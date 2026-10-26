package main

import (
	"testing"
	"time"

	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/login"
)

// Real player teardown must work with no Experience host, regardless of which peer closes first.
func TestServerStopsWithoutExperiencesWithConnectedPlayer(t *testing.T) {
	for _, clientClosesFirst := range []bool{false, true} {
		name := "server_closes_first"
		if clientClosesFirst {
			name = "client_closes_first"
		}
		t.Run(name, func(t *testing.T) {
			addr := freeAddr(t)
			stop := startServer(t, []string{"-dir", t.TempDir(), "-addr", addr})
			conn, err := (minecraft.Dialer{IdentityData: login.IdentityData{DisplayName: "ShutdownFixture"}}).DialTimeout("raknet", addr, 20*time.Second)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { _ = conn.Close() })
			if err := conn.DoSpawn(); err != nil {
				t.Fatal(err)
			}
			if clientClosesFirst {
				_ = conn.Close()
			}
			stop()
		})
	}
}
