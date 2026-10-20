//! Controlled ordinary friend actions; the presenter fixture is not a saved-auth or retail-friend claim.

mod fixture;

use super::*;
use fixture::Original;
use std::time::Duration;

/// Ordinary current rows must reach signed game initialization and original conditional leave for both dialects.
#[test]
fn controlled_native_friend_play_initializes_both_dialects_and_leaves_once() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let outcomes = runtime.block_on(async {
        let mut outcomes = Vec::new();
        for legacy in [false, true] {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut original = Original::open(legacy, deadline).await;
            let host_xuid = original
                .app
                .world()
                .resource::<MenuRuntime>()
                .view()
                .friends[0]
                .xuid
                .clone();
            let queued = original.play();
            if !queued {
                let (_, joined) = original.finish(false).await;
                outcomes.push((queued, false, false, joined, None));
                continue;
            }
            let host = original.service.listen(&host_xuid, deadline).await.unwrap();
            let (initialized, observed) = tokio::sync::oneshot::channel();
            let (marker, received_marker) = tokio::sync::oneshot::channel();
            let peer = tokio::spawn(host.serve(initialized, marker, deadline));
            original
                .service
                .joined()
                .await
                .expect("actual original membership response");
            original.service.admit();
            let bootstrap = original.bootstrap(deadline).await;
            let initialized = tokio::time::timeout_at(deadline.into(), observed)
                .await
                .is_ok_and(|result| result.is_ok());
            let marker_sent = if initialized {
                let network = original
                    .app
                    .world()
                    .resource::<NetworkHandle<PackApplication>>();
                let result = network
                    .send_packet(protocol::request_chunk_radius_packet(
                        protocol::test_support::friend_peer::MARKER_RADIUS,
                        protocol::test_support::friend_peer::MARKER_RADIUS,
                    ))
                    .is_ok();
                network.flush_frame();
                result
            } else {
                false
            };
            let marker_received = if marker_sent {
                tokio::time::timeout_at(deadline.into(), received_marker)
                    .await
                    .is_ok_and(|result| result.is_ok())
            } else {
                false
            };
            original.disconnect();
            let retired = original.retired(deadline).await;
            let peer = peer.await.unwrap();
            let (facts, joined) = original.finish(retired).await;
            let once = facts.is_some_and(|facts| {
                facts.joins == 1
                    && facts.leaves == 1
                    && facts.nonce_published
                    && facts.minted
                    && facts.signaling_closed
                    && facts.rta_retired
            });
            outcomes.push((
                queued,
                bootstrap && initialized,
                marker_received,
                retired && once && peer.retired.is_ok() && joined,
                Some(peer.play.map_err(|error| error.kind())),
            ));
        }
        outcomes
    });
    // Both dialects' original peer/service/worker lifetimes have ended before this capability assertion.
    assert!(
        outcomes.iter().all(|(_, _, _, retired, _)| *retired),
        "original friend owners did not retire: {outcomes:?}"
    );
    assert!(
        outcomes.iter().all(|(queued, _, _, _, _)| *queued),
        "ordinary current native friend rows are still refused"
    );
    assert!(
        outcomes
            .iter()
            .all(|(_, initialized, marker, _, play)| *initialized
                && *marker
                && matches!(play, Some(Ok(true)))),
        "signed original friend game did not initialize, preserve its marker or close cleanly: {outcomes:?}"
    );
}

/// Cancel after the actual member response, before nonce publication; the same companion must leave once.
#[test]
fn controlled_native_friend_cancel_after_membership_retires_both_dialects() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let outcomes = runtime.block_on(async {
        let mut outcomes = Vec::new();
        for legacy in [false, true] {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut original = Original::open(legacy, deadline).await;
            let queued = original.play();
            if !queued {
                let (_, joined) = original.finish(false).await;
                outcomes.push((queued, false, joined));
                continue;
            }
            original
                .service
                .joined()
                .await
                .expect("actual original membership response");
            original.cancel();
            let retired = original.retired(deadline).await;
            let (observed, joined) = original.finish(retired).await;
            let exact = observed.is_some_and(|observed| {
                observed.joins == 1
                    && observed.leaves == 1
                    && !observed.nonce_published
                    && !observed.minted
                    && !observed.signaling_started
                    && observed.rta_retired
            });
            outcomes.push((queued, exact, retired && joined));
        }
        outcomes
    });
    assert!(
        outcomes.iter().all(|(_, _, retired)| *retired),
        "original canceled friend owners did not retire"
    );
    assert!(
        outcomes.iter().all(|(queued, _, _)| *queued),
        "ordinary current native friend rows are still refused"
    );
    assert!(
        outcomes.iter().all(|(_, exact, _)| *exact),
        "member cancellation minted transport, lost membership or replayed its join/leave"
    );
}
