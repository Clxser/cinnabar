//! Featured servers: the Cinnabar API's curated list, cached for offline starts, followed by the
//! snapshot the game saves after its account services answer.

use anyhow::Result;
use launcher::desktop::{FeaturedEntry, FeaturedSnapshot};
use serde::{Deserialize, Serialize};
use std::path::Path;

const ROUTE: &str = "/v1/featured";
const MAX_CURATED: usize = 24;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FeaturedServer {
    pub id: String,
    pub name: String,
    /// A host name, or an experience ID joined with port 0.
    pub address: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub online: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub motd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub tags: Option<Vec<String>>,
    /// Round trip in milliseconds; the launcher does not measure it yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub ping_ms: Option<u32>,
}

#[derive(Deserialize, Serialize)]
struct Curated {
    servers: Vec<FeaturedServer>,
}

/// The servers the game last saw; empty until the game has signed in once.
pub fn from_game(path: &Path) -> Vec<FeaturedServer> {
    FeaturedSnapshot::load(path)
        .servers
        .into_iter()
        .filter(|entry| !entry.address.is_empty())
        .map(server)
        .collect()
}

/// Keeps well-formed curated entries; HTTPS artwork only, and no launcher-measured fields.
pub fn parse(body: &[u8]) -> Result<Vec<FeaturedServer>> {
    let curated: Curated = serde_json::from_slice(body)?;
    Ok(curated
        .servers
        .into_iter()
        .filter(|server| !server.id.is_empty() && !server.address.is_empty())
        .map(|server| FeaturedServer {
            image_url: server.image_url.filter(|url| url.starts_with("https://")),
            online: None,
            ping_ms: None,
            ..server
        })
        .take(MAX_CURATED)
        .collect())
}

/// Fetches the curated list from the API.
pub async fn fetch() -> Result<Vec<FeaturedServer>> {
    parse(&crate::api::get(ROUTE).await?)
}

pub fn load_cache(path: &Path) -> Option<Vec<FeaturedServer>> {
    let curated: Curated = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(curated.servers)
}

pub fn save_cache(path: &Path, servers: &[FeaturedServer]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("featured cache has no directory"))?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(
        &mut file,
        &Curated {
            servers: servers.to_vec(),
        },
    )?;
    file.persist(path)?;
    Ok(())
}

/// The curated servers first, then the game's servers that the curated list lacks.
pub fn merge(curated: Vec<FeaturedServer>, game: Vec<FeaturedServer>) -> Vec<FeaturedServer> {
    let mut servers = curated;
    for server in game {
        if !servers.iter().any(|known| {
            known.id == server.id || known.address == server.address && known.port == server.port
        }) {
            servers.push(server);
        }
    }
    servers
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
        motd: (!entry.motd.is_empty()).then_some(entry.motd),
        tags: Some(
            entry
                .tags
                .into_iter()
                .filter(|tag| !tag.is_empty())
                .collect::<Vec<_>>(),
        )
        .filter(|tags| !tags.is_empty()),
        ping_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_servers_come_first_and_the_game_adds_only_new_ones() {
        let body = serde_json::json!({ "servers": [
            { "id": "a", "name": "A", "address": "a.example.test", "port": 19132,
              "imageUrl": "http://insecure.example.test/a.png", "tags": ["PvP"] },
            { "id": "", "name": "No id", "address": "x.example.test", "port": 1 },
        ]});
        let curated = parse(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(curated.len(), 1);
        assert_eq!(curated[0].image_url, None);
        assert!(parse(b"{").is_err());
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("featured.json");
        save_cache(&cache, &curated).unwrap();
        assert_eq!(load_cache(&cache), Some(curated.clone()));
        let game = |id: &str, address: &str| FeaturedServer {
            id: id.into(),
            name: id.into(),
            address: address.into(),
            port: 19132,
            description: None,
            image_url: None,
            online: Some(3),
            motd: None,
            tags: None,
            ping_ms: None,
        };
        let merged = merge(
            curated,
            vec![
                game("a.example.test:19132", "a.example.test"),
                game("b", "b.example.test"),
            ],
        );
        assert_eq!(
            merged
                .iter()
                .map(|server| server.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

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
                    motd: "Season 3".into(),
                    tags: vec!["Bedwars".into(), String::new()],
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
        let servers = from_game(&path);
        assert_eq!(servers.len(), 2);
        assert_eq!(
            (servers[0].address.as_str(), servers[0].port),
            ("play.example.test", 19132)
        );
        assert_eq!(servers[0].online, Some(42));
        assert_eq!(servers[0].description.as_deref(), Some("Minigames"));
        assert_eq!(servers[0].motd.as_deref(), Some("Season 3"));
        assert_eq!(
            servers[0].tags.as_deref(),
            Some(&["Bedwars".to_owned()][..])
        );
        assert_eq!(
            (servers[1].address.as_str(), servers[1].port),
            (experience.as_str(), 0)
        );
        assert_eq!(servers[1].image_url, None);
        assert_eq!(servers[1].online, None);
    }
}
