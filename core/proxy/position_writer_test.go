package proxy

import (
	"context"
	"errors"
	"io"
	"testing"
	"testing/synctest"

	"github.com/go-gl/mathgl/mgl32"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

func TestPacketDelayPositionPublishesIdleInterceptorWrites(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		delay := new(PacketDelay)
		id := delay.beginSession()
		_ = delay.SetWithPosition(200, true)
		up := newFakeUpstream(nil)
		observed := observeUpstreamPositions(up, delay, 9)
		intercepted := &InterceptedSession{server: observed}
		src := newFakeDownstream(nil)
		defer src.Close()
		src.useBatchReads = true
		done := make(chan error, 1)
		go func() { done <- pumpPacketsWithDelay(context.Background(), delay, nil, src, observed, true, id) }()
		synctest.Wait()
		if err := intercepted.WriteToServer(&packet.PlayerAuthInput{Position: mgl32.Vec3{1, 70, 3}}); err != nil {
			t.Fatal(err)
		}
		if _, position := delay.PositionSnapshot(); position != nil {
			t.Fatal("an unflushed injection advanced the position")
		}
		src.batchReads <- batchResult{err: io.EOF}
		if err := <-done; !errors.Is(err, io.EOF) {
			t.Fatal(err)
		}
		if _, position := delay.PositionSnapshot(); position == nil || *position != (ForwardedPosition{1, 70, 3}) {
			t.Fatalf("idle flush position = %#v", position)
		}
		if batches := up.flushedBatches(); len(batches) != 1 || len(batches[0]) != 1 {
			t.Fatalf("idle flush batches = %#v", batches)
		}
	})
}

func TestPacketDelayPositionFencesQueuedInterceptorWrites(t *testing.T) {
	for _, action := range []string{"lease", "session", "failed flush"} {
		t.Run(action, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				delay := new(PacketDelay)
				id := delay.beginSession()
				_ = delay.SetWithPosition(200, true)
				up := newFakeUpstream(nil)
				observed := observeUpstreamPositions(up, delay, 9)
				trackPositionWrites(observed, delay, id, 9)
				intercepted := &InterceptedSession{server: observed}
				if err := intercepted.WriteToServer(&packet.PlayerAuthInput{Position: mgl32.Vec3{1, 70, 3}}); err != nil {
					t.Fatal(err)
				}
				failure := errors.New("flush rejected")
				switch action {
				case "lease":
					_ = delay.SetWithPosition(200, false)
					_ = delay.SetWithPosition(200, true)
				case "session":
					delay.beginSession()
				case "failed flush":
					up.setFlushErr(failure)
				}
				err := intercepted.FlushServer()
				if action == "failed flush" && !errors.Is(err, failure) || action != "failed flush" && err != nil {
					t.Fatalf("flush = %v", err)
				}
				if _, position := delay.PositionSnapshot(); position != nil {
					t.Fatal("a stale or unflushed injection advanced the position")
				}
				if action == "failed flush" {
					up.setFlushErr(nil)
					if err := intercepted.FlushServer(); err != nil {
						t.Fatal(err)
					}
					if _, position := delay.PositionSnapshot(); position == nil || position.X != 1 {
						t.Fatal("a successful retry lost the queued movement")
					}
				}
			})
		})
	}
}
