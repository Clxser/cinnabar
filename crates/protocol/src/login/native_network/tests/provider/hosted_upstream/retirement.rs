//! Physical completion of the original upstream after its game input has been consumed.

use super::*;
use std::{io, net::UdpSocket, task::Poll};

#[derive(Debug)]
pub(super) struct Observation {
    initially_pending: bool,
    socket_held_on_first_poll: bool,
    finish: Option<io::ErrorKind>,
    socket_released_at_completion: bool,
    trailing_packets: usize,
    terminal: io::ErrorKind,
    socket_released_after_drop: bool,
}

/// Keeps the same closing owner alive while observing its actual socket's lifetime.
pub(super) async fn observe(
    upstream: NativeHostedUpstream,
    deadline: Instant,
    address: std::net::SocketAddr,
) -> Observation {
    let mut closing = upstream.begin_close(deadline);
    let (initially_pending, socket_held_on_first_poll, finish) = {
        let mut finish = std::pin::pin!(closing.finish_close());
        let first = futures::poll!(finish.as_mut());
        // Current-thread execution has not yielded since the first close poll.
        let socket_held =
            UdpSocket::bind(address).is_err_and(|error| error.kind() == io::ErrorKind::AddrInUse);
        let (pending, result) = match first {
            Poll::Pending => (true, finish.await),
            Poll::Ready(result) => (false, result),
        };
        (pending, socket_held, result.err().map(|error| error.kind()))
    };
    let socket_released_at_completion = UdpSocket::bind(address).is_ok();
    // The request-only baseline still owns the socket here. Finish its original input
    // before checking the oracle, without treating this locally requested EOF as peer EOF.
    let mut trailing_packets = 0;
    let terminal = loop {
        match closing.receive().await {
            Ok(_) => trailing_packets += 1,
            Err(error) => break error.kind(),
        }
    };
    drop(closing);
    let socket_released_after_drop = UdpSocket::bind(address).is_ok();
    Observation {
        initially_pending,
        socket_held_on_first_poll,
        finish,
        socket_released_at_completion,
        trailing_packets,
        terminal,
        socket_released_after_drop,
    }
}

/// The real initialized upstream must retire physically before final close returns.
#[tokio::test]
async fn hosted_upstream_finish_close_waits_for_its_original_socket_retirement() {
    let observed = tokio::time::timeout(
        TIMEOUT,
        original_upstream_with_retirement(
            Route::LocalHttp,
            None,
            archive_bytes(PACK_ID, &[]),
            bridge::contract::RESOURCE_PACK_TYPE_RESOURCES,
            true,
        ),
    )
    .await
    .expect("finite original upstream retirement fixture stalled")
    .unwrap();
    assert_eq!(observed.finish, None, "{observed:?}");
    assert!(observed.socket_held_on_first_poll, "{observed:?}");
    assert_eq!(observed.trailing_packets, 0, "{observed:?}");
    assert_eq!(
        observed.terminal,
        io::ErrorKind::UnexpectedEof,
        "{observed:?}"
    );
    assert!(observed.socket_released_after_drop, "{observed:?}");
    assert!(
        observed.initially_pending && observed.socket_released_at_completion,
        "finish_close returned before its original upstream socket retired: {observed:?}"
    );
}
