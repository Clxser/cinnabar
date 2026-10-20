//! Actual generated local Login/startup traffic with no ResourcePacksInfo or ResourcePackStack.

use super::*;
use bedrock_net::{BatchPhase, CompressionAlgorithm, CompressionSettings};
use bedrock_protocol_1_26_51::{
    enums::{PlayStatusType, ServerboundLoadingScreenType},
    packets,
    types::ActorRuntimeID,
    wire,
};
use nethernet_tokio::AcceptedSession;

pub(super) const RUNTIME_ID: u64 = 42;
pub(super) const MARKER_RADIUS: i32 = 3;

/// Sends the same original typed packets through the selected native batch codec.
async fn send<S: BatchPhase>(
    peer: &AcceptedSession,
    codec: &BatchCodec<S>,
    packets: &[packets::Packet],
) -> io::Result<()> {
    let mut entries = Vec::with_capacity(packets.len());
    for packet in packets {
        let mut frame = Vec::new();
        packet
            .encode_frame(wire::Peer::Server, 0, 0, &mut frame)
            .map_err(io::Error::other)?;
        entries.push(Bytes::from(frame));
    }
    for message in codec.encode(&entries).map_err(io::Error::other)? {
        peer.session.send(message).await.map_err(io::Error::other)?;
    }
    Ok(())
}

/// Decodes complete original client entries; malformed and unknown input remains a fixture failure.
async fn receive<S: BatchPhase>(
    peer: &mut AcceptedSession,
    codec: &BatchCodec<S>,
) -> io::Result<Vec<packets::Packet>> {
    let message = peer
        .reliable
        .recv()
        .await
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
    let DecodeOutcome::Packets { packets, .. } = codec.decode(message).map_err(io::Error::other)?
    else {
        return Err(io::Error::other(
            "packless fixture received a dropped batch",
        ));
    };
    packets
        .map(|entry| {
            let mut reader = wire::Reader::from_shared(&entry);
            let header =
                wire::PacketHeader::from_raw(reader.read_var_u32().map_err(io::Error::other)?);
            let id = packets::PacketId::from_raw(header.id)
                .ok_or_else(|| io::Error::other("packless fixture received unknown input"))?;
            let packet = packets::Packet::decode_body(id, &mut reader).map_err(io::Error::other)?;
            reader.expect_consumed().map_err(io::Error::other)?;
            Ok(packet)
        })
        .collect()
}

/// Requires real settings/Login, request/spawn, Initialized and a later play round trip.
pub(super) async fn run(
    peer: &mut AcceptedSession,
    startup_sent: tokio::sync::oneshot::Sender<()>,
) -> io::Result<()> {
    let codec = BatchCodec::new(BatchHeader::NetherNet, BatchLimits::default());
    if !matches!(
        receive(peer, &codec).await?.as_slice(),
        [packets::Packet::RequestNetworkSettings(_)]
    ) {
        return Err(io::Error::other("packless upstream expected settings"));
    }
    send(
        peer,
        &codec,
        &[packets::NetworkSettings {
            compression_threshold: wire::U16LE(u16::MAX),
            ..Default::default()
        }
        .into()],
    )
    .await?;
    let codec = codec.negotiate(CompressionSettings {
        algorithm: CompressionAlgorithm::Deflate,
        threshold: u16::MAX,
    });
    if !matches!(
        receive(peer, &codec).await?.as_slice(),
        [packets::Packet::Login(_)]
    ) {
        return Err(io::Error::other(
            "packless upstream expected original Login",
        ));
    }
    send(
        peer,
        &codec,
        &[
            packets::PlayStatus {
                status: PlayStatusType::Loginsuccess,
            }
            .into(),
            packets::SetTime {
                time: wire::ZigZag32(1),
            }
            .into(),
            packets::StartGame {
                runtime_id: ActorRuntimeID(RUNTIME_ID),
                ..Default::default()
            }
            .into(),
            packets::ItemRegistry::default().into(),
            packets::SetTime {
                time: wire::ZigZag32(2),
            }
            .into(),
        ],
    )
    .await?;
    let _ = startup_sent.send(());
    let mut requested = false;
    let mut loading = false;
    while !(requested && loading) {
        for packet in receive(peer, &codec).await? {
            match packet {
                packets::Packet::ClientCacheStatus(_) => {}
                packets::Packet::RequestChunkRadius(request)
                    if !requested && request.chunk_radius.0 == 2 =>
                {
                    requested = true
                }
                packets::Packet::ServerboundLoadingScreen(screen)
                    if !loading
                        && screen.loading_screen_packet_type
                            == ServerboundLoadingScreenType::Startloadingscreen =>
                {
                    loading = true
                }
                _ => {
                    return Err(io::Error::other(
                        "packless upstream received invented pack traffic or changed startup",
                    ));
                }
            }
        }
    }
    send(
        peer,
        &codec,
        &[
            packets::ChunkRadiusUpdated {
                chunk_radius: wire::ZigZag32(2),
            }
            .into(),
            packets::PlayStatus {
                status: PlayStatusType::Playerspawn,
            }
            .into(),
        ],
    )
    .await?;
    let mut initialized = false;
    let mut loading_done = false;
    while !(initialized && loading_done) {
        for packet in receive(peer, &codec).await? {
            match packet {
                packets::Packet::ServerboundLoadingScreen(screen)
                    if !loading_done
                        && screen.loading_screen_packet_type
                            == ServerboundLoadingScreenType::Endloadingscreen =>
                {
                    loading_done = true
                }
                packets::Packet::SetLocalPlayerAsInitialized(packet)
                    if !initialized && packet.player_id.0 == RUNTIME_ID =>
                {
                    initialized = true
                }
                _ => {
                    return Err(io::Error::other(
                        "packless upstream lost actual initialization order",
                    ));
                }
            }
        }
    }
    if !matches!(receive(peer, &codec).await?.as_slice(),
        [packets::Packet::RequestChunkRadius(request)] if request.chunk_radius.0 == MARKER_RADIUS)
    {
        return Err(io::Error::other("packless upstream lost live marker"));
    }
    send(
        peer,
        &codec,
        &[packets::ChunkRadiusUpdated {
            chunk_radius: wire::ZigZag32(MARKER_RADIUS),
        }
        .into()],
    )
    .await?;
    // The original visitor must close normally after its actual marker response.
    match peer.reliable.recv().await {
        Ok(None) => Ok(()),
        result => Err(io::Error::other(format!(
            "packless upstream expected original EOF: {}",
            result.is_ok()
        ))),
    }
}
