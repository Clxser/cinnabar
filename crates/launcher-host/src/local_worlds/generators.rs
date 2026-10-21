//! Discovery cached by executable identity, shared by every manager in this host.

use super::{
    lock,
    world::{WorldError, failed},
};
use launcher::local_worlds::generators::{GeneratorInfo, parse_catalog};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, SystemTime},
};

#[derive(Clone)]
struct Cached {
    modified: SystemTime,
    catalog: Result<Vec<GeneratorInfo>, String>,
}

/// Reads -list-generators once per path and modification time, including failures.
pub(super) fn catalog(binary: &Path) -> Result<Vec<GeneratorInfo>, WorldError> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    let path = binary
        .canonicalize()
        .map_err(|e| failed(format!("find local server: {e}")))?;
    let modified = path
        .metadata()
        .and_then(|m| m.modified())
        .map_err(|e| failed(format!("stat local server: {e}")))?;
    let mut cache = lock(CACHE.get_or_init(Mutex::default));
    let entry = cache.entry(path.clone()).or_insert_with(|| Cached {
        modified,
        catalog: discover(&path),
    });
    if entry.modified != modified {
        *entry = Cached {
            modified,
            catalog: discover(&path),
        };
    }
    entry.catalog.clone().map_err(|detail| {
        tracing::error!("local server generator discovery failed: {detail}");
        WorldError::GeneratorCatalog
    })
}

/// Runs catalog discovery without inheriting input or mixing diagnostics into the JSON.
fn discover(binary: &Path) -> Result<Vec<GeneratorInfo>, String> {
    let mut command = Command::new(binary);
    command
        .arg("-list-generators")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child =
        Arc::new(crate::lifecycle::children::spawn(&mut command).map_err(|e| e.to_string())?);
    let mut stdout = child.take_stdout().expect("piped catalog output");
    let (sent, received) = crossbeam_channel::bounded(1);
    let watched = Arc::clone(&child);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.read_to_end(&mut bytes);
        let status = watched.wait();
        let _ = sent.send((read, status, bytes));
    });
    let (read, status, bytes) = match received.recv_timeout(Duration::from_secs(30)) {
        Ok(output) => output,
        Err(_) => {
            child.kill();
            let _ = child.wait();
            return Err("-list-generators did not finish in time".into());
        }
    };
    read.map_err(|e| e.to_string())?;
    if !status.is_some_and(|status| status.success()) {
        return Err(format!("-list-generators failed: {status:?}"));
    }
    parse_catalog(&bytes)
}

/// Resolves an explicit executable override only for a development build. A relative override
/// is anchored to the working directory so discovery and startup run the same file, never a
/// `PATH` lookup.
fn select_binary(
    shipped: PathBuf,
    override_path: Option<std::ffi::OsString>,
    development: bool,
) -> PathBuf {
    if development && let Some(path) = override_path {
        let path = PathBuf::from(path);
        return std::path::absolute(&path).unwrap_or(path);
    }
    shipped
}

/// Release builds never read the local-server override from the environment.
pub(super) fn server_binary(shipped: PathBuf) -> PathBuf {
    #[cfg(debug_assertions)]
    let override_path = std::env::var_os("CINNABAR_LOCAL_SERVER");
    #[cfg(not(debug_assertions))]
    let override_path = None;
    select_binary(shipped, override_path, cfg!(debug_assertions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_configuration_ignores_override() {
        let shipped = PathBuf::from("shipped");
        assert_eq!(
            select_binary(shipped.clone(), Some("custom".into()), false),
            shipped
        );
        let selected = select_binary(shipped, Some("custom".into()), true);
        assert!(selected.is_absolute());
        assert_eq!(selected, std::env::current_dir().unwrap().join("custom"));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_is_cached_by_path_and_mtime() {
        use crate::local_worlds::test_support::write_script;
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("server");
        write_script(&binary, b"#!/bin/sh\necho called >> \"$0.calls\"\necho '[{\"id\":\"islands\",\"label\":\"Islands\"}]'\n");
        assert_eq!(catalog(&binary).unwrap()[0].id.as_str(), "islands");
        catalog(&binary).unwrap();
        assert_eq!(
            std::fs::read_to_string(binary.with_extension("calls")).unwrap(),
            "called\n"
        );
        let stamp = binary.metadata().unwrap().modified().unwrap();
        write_script(&binary, b"#!/bin/sh\necho called >> \"$0.calls\"\necho '[{\"id\":\"void\",\"label\":\"Void\"}]'\n");
        std::fs::File::options()
            .write(true)
            .open(&binary)
            .unwrap()
            .set_modified(stamp + std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(catalog(&binary).unwrap()[0].id.as_str(), "void");
        assert_eq!(
            std::fs::read_to_string(binary.with_extension("calls")).unwrap(),
            "called\ncalled\n"
        );
        let other = dir.path().join("other");
        write_script(
            &other,
            b"#!/bin/sh\necho '[{\"id\":\"other\",\"label\":\"Other\"}]'\n",
        );
        assert_eq!(catalog(&other).unwrap()[0].id.as_str(), "other");
    }
}
