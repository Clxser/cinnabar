use std::collections::VecDeque;
use std::sync::{Mutex as StdMutex, PoisonError};

use bedrock_protocol_1_26_51::packets::{self, SetTime};
use bedrock_protocol_1_26_51::wire;
use tokio::io::AsyncReadExt;

use super::*;
use crate::Packet;
use crate::session_transport::tests::unix::{Pair, pair, write_frame};
use crate::session_transport::{Inbound, batch_frame};

/// A relay that records what it sees and answers from scripted replies; an empty script forwards.
#[derive(Default)]
pub(crate) struct FakeRelay {
    pub(crate) state: StdMutex<FakeState>,
}

#[derive(Default)]
pub(crate) struct FakeState {
    pub(crate) started: Option<(SessionIdentity, String, Vec<u8>)>,
    pub(crate) packs: Vec<(Vec<u8>, bool)>,
    pub(crate) inbound: Vec<Vec<u8>>,
    pub(crate) startup_inbound: Vec<Vec<u8>>,
    pub(crate) inbound_replies: VecDeque<Option<Vec<u8>>>,
    pub(crate) outbound: Vec<Vec<u8>>,
    pub(crate) outbound_replies: VecDeque<Option<Vec<u8>>>,
    pub(crate) ticks: VecDeque<RelayOutput>,
    pub(crate) stopped: bool,
}

impl FakeRelay {
    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PacketRelay for FakeRelay {
    fn start(
        &self,
        identity: &SessionIdentity,
        client_data: &str,
        startup: &[u8],
    ) -> Vec<RelayPack> {
        let mut state = self.state();
        state.started = Some((identity.clone(), client_data.to_owned(), startup.to_vec()));
        state
            .packs
            .iter()
            .map(|(archive, above_server)| RelayPack {
                archive: archive.clone(),
                above_server: *above_server,
            })
            .collect()
    }
    fn inbound_startup(&self, body: &[u8]) -> Option<Vec<u8>> {
        self.state().startup_inbound.push(body.to_vec());
        self.inbound(body)
    }

    fn inbound(&self, body: &[u8]) -> Option<Vec<u8>> {
        let mut state = self.state();
        state.inbound.push(body.to_vec());
        state.inbound_replies.pop_front().flatten()
    }
    fn outbound(&self, body: &[u8]) -> Option<Vec<u8>> {
        let mut state = self.state();
        state.outbound.push(body.to_vec());
        state.outbound_replies.pop_front().flatten()
    }
    fn tick(&self) -> RelayOutput {
        self.state().ticks.pop_front().unwrap_or_default()
    }
    fn stop(&self) {
        self.state().stopped = true;
    }
}

fn set_time(time: i32) -> Packet {
    Packet::new(SetTime {
        time: wire::ZigZag32(time),
    })
}

/// The batch body carrying `times` as SetTime packets.
pub(crate) fn body(times: &[i32]) -> Vec<u8> {
    let packets: Vec<_> = times.iter().copied().map(set_time).collect();
    batch_frame(&packets).unwrap()[1..].to_vec()
}

async fn relayed_pair(name: &str, relay: &Arc<FakeRelay>) -> Pair {
    let mut pair = pair(name, &[]).await;
    pair.transport.set_relay(relay.clone());
    pair
}

/// Reads the SetTime values of the next `count` packets.
async fn next_times(pair: &mut Pair, count: usize) -> Vec<i32> {
    let mut times = Vec::new();
    for _ in 0..count {
        let Inbound::Packet(raw) = pair.transport.recv().await.unwrap() else {
            panic!("a packet arrives before any terminal message")
        };
        let packets::Packet::SetTime(time) = *raw.decode().unwrap().data else {
            panic!("only SetTime packets are relayed")
        };
        times.push(time.time.0);
    }
    times
}

/// Reads one frame the transport wrote to the core.
async fn read_frame(pair: &mut Pair) -> Vec<u8> {
    let length = pair.peer.read_u32().await.unwrap();
    let mut frame = vec![0; length as usize];
    pair.peer.read_exact(&mut frame).await.unwrap();
    frame
}

/// Server batches pass through, are replaced or dropped, and a replacement that does not decode
/// is dropped without ending the session.
#[tokio::test]
async fn server_batches_pass_through_get_replaced_or_dropped() {
    let relay = Arc::new(FakeRelay::default());
    relay.state().inbound_replies = [
        None,
        Some(body(&[7])),
        Some(Vec::new()),
        Some(vec![0xff]),
        None,
    ]
    .into();
    let mut pair = relayed_pair("relay-inbound", &relay).await;
    for time in 1..=5 {
        write_frame(&mut pair.peer, &batch_frame(&[set_time(time)]).unwrap()).await;
    }
    assert_eq!(next_times(&mut pair, 3).await, [1, 7, 5]);
    let seen = relay.state().inbound.clone();
    assert_eq!(seen, (1..=5).map(|time| body(&[time])).collect::<Vec<_>>());
}

/// Client batches reach the core as the relay leaves them; a dropped batch writes nothing.
#[tokio::test]
async fn client_batches_are_relayed_in_write_order() {
    let relay = Arc::new(FakeRelay::default());
    relay.state().outbound_replies = [Some(body(&[5])), Some(Vec::new()), None].into();
    let mut pair = relayed_pair("relay-outbound", &relay).await;
    for time in 1..=3 {
        pair.transport.send(&[set_time(time)]).await.unwrap();
    }
    for expected in [5, 3] {
        let mut frame = vec![bridge::contract::KIND_BATCH];
        frame.extend_from_slice(&body(&[expected]));
        assert_eq!(read_frame(&mut pair).await, frame);
    }
    let seen = relay.state().outbound.clone();
    assert_eq!(seen, (1..=3).map(|time| body(&[time])).collect::<Vec<_>>());
}

/// Timed work injects bodies both ways, and dropping the session stops the relay.
#[tokio::test]
async fn ticks_inject_both_ways_and_the_session_stops_the_relay() {
    let relay = Arc::new(FakeRelay::default());
    relay.state().ticks = [RelayOutput {
        to_client: body(&[8]),
        to_server: body(&[9]),
    }]
    .into();
    let mut pair = relayed_pair("relay-tick", &relay).await;
    assert_eq!(next_times(&mut pair, 1).await, [8]);
    let mut frame = vec![bridge::contract::KIND_BATCH];
    frame.extend_from_slice(&body(&[9]));
    assert_eq!(read_frame(&mut pair).await, frame);
    assert!(!relay.state().stopped);
    drop(pair);
    assert!(relay.state().stopped);
}

/// The startup body carries each packet behind its varuint32 length.
#[test]
fn startup_body_joins_packets_with_length_prefixes() {
    let packets = [Bytes::from_static(&[1, 2]), Bytes::from(vec![3; 200])];
    let body = batch_body(&packets);
    let mut expected = vec![2, 1, 2, 200, 1];
    expected.extend_from_slice(&[3; 200]);
    assert_eq!(&body[..], &expected[..]);
}
