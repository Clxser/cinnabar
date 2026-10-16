//! Featured servers from the snapshot the game saves after its account services answer.

use launcher::desktop::{FeaturedEntry, FeaturedSnapshot};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FeaturedServer {
    pub id: String,
    pub name: String,
    /// A host name, or an experience ID joined with port 0.
    pub address: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub online: Option<u32>,
}

/// The servers the game last saw; empty until the game has signed in once.
pub fn load(path: &Path) -> Vec<FeaturedServer> {
    FeaturedSnapshot::load(path)
        .servers
        .into_iter()
        .filter(|entry| !entry.address.is_empty())
        .map(server)
        .collect()
}

fn server(entry: FeaturedEntry) -> FeaturedServer {
    let (address, port) = entry
        .address
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host.to_owned(), port.parse().ok()?)))
        .filter(|(host, _)| !host.is_empty())
        .unwrap_or_else(|| (entry.address.clone(), 0));
    FeaturedServer {
        id: entry.address,
        name: entry.name,
        address,
        port,
        description: (!entry.caption.is_empty()).then_some(entry.caption),
        image_url: entry
            .logo_url
            .starts_with("https://")
            .then_some(entry.logo_url),
        online: entry
            .player_count
            .and_then(|count| u32::try_from(count).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_split_from_ports_and_experiences_keep_their_id() {
        let experience = format!("{}abc", launcher::menu::EXPERIENCE_ADDRESS_PREFIX);
        let snapshot = FeaturedSnapshot {
            servers: vec![
                FeaturedEntry {
                    name: "Server".into(),
                    address: "play.example.test:19132".into(),
                    caption: "Minigames".into(),
                    logo_url: "https://example.test/logo.png".into(),
                    player_count: Some(42),
                },
                FeaturedEntry {
                    name: "Experience".into(),
                    address: experience.clone(),
                    logo_url: "http://example.test/insecure.png".into(),
                    player_count: Some(-1),
                    ..FeaturedEntry::default()
                },
                FeaturedEntry::default(),
            ],
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("featured.json");
        snapshot.save(&path).unwrap();
        let servers = load(&path);
        assert_eq!(servers.len(), 2);
        assert_eq!(
            (servers[0].address.as_str(), servers[0].port),
            ("play.example.test", 19132)
        );
        assert_eq!(servers[0].online, Some(42));
        assert_eq!(servers[0].description.as_deref(), Some("Minigames"));
        assert_eq!((servers[1].address.as_str(), servers[1].port), (experience.as_str(), 0));
        assert_eq!(servers[1].image_url, None);
        assert_eq!(servers[1].online, None);
    }
}
