//! Local world data shared with the native hosting owner.

use crate::contract::{Backend, GameMode, Transport, WorldState};
use serde::Serialize;

/// The open world's loopback server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalTarget {
    pub address: String,
    pub transport: Transport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lan_address: Option<String>,
    /// The BDS level its LAN advertisement must name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_name: Option<String>,
}

/// The open world's identity advertised to Xbox friends.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalWorldInfo {
    pub id: String,
    pub name: String,
    pub game_mode: GameMode,
    pub backend: Backend,
}

/// The launcher's open local world.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct LocalWorld {
    pub state: WorldState,
    /// Set unless idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world: Option<LocalWorldInfo>,
    /// Set while running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<LocalTarget>,
    /// Path-free reason while failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_reports_only_the_state() {
        let idle = serde_json::to_value(LocalWorld::default()).expect("encode");
        assert_eq!(idle, serde_json::json!({"state": "idle"}));
    }

    #[test]
    fn running_reports_name_the_world_and_its_transport() {
        let running = LocalWorld {
            state: WorldState::Running,
            world: Some(LocalWorldInfo {
                id: "0123456789abcdef".into(),
                name: "Home".into(),
                game_mode: GameMode::Creative,
                backend: Backend::Bds,
            }),
            target: Some(LocalTarget {
                address: "127.0.0.1:5000".into(),
                transport: Transport::NetherNetLan,
                lan_address: Some("127.0.0.1:7551".into()),
                level_name: Some("0123456789abcdef".into()),
            }),
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&running).expect("encode"),
            serde_json::json!({
                "state": "running",
                "world": {"id": "0123456789abcdef", "name": "Home", "game_mode": "creative", "backend": "bds"},
                "target": {"address": "127.0.0.1:5000", "transport": "nethernet-lan",
                           "lan_address": "127.0.0.1:7551", "level_name": "0123456789abcdef"},
            })
        );
        let raknet = serde_json::to_value(Transport::RakNet).expect("encode");
        assert_eq!(raknet, serde_json::json!("raknet"));
        let http = serde_json::to_value(Transport::NetherNet).expect("encode");
        assert_eq!(http, serde_json::json!("nethernet"));
    }
}
