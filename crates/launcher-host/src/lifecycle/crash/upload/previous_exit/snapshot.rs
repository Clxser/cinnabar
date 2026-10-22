//! Previous-session data retained before Android allows another game launch to rotate logs.

use crash_report::{
    folder::{self, LogSources, Logs},
    section,
    upload::state,
};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Reports and bounded log bytes captured before the healthy launch starts a new game session.
pub struct Snapshot {
    pub(super) logs: Logs,
    pub(super) folders: Vec<PathBuf>,
    saved: Vec<PathBuf>,
}

impl Snapshot {
    /// Captures immutable previous-session data before Retry or startup can change it.
    /// The caller runs this once at process startup, before starting any game activity.
    pub fn capture(crash_dir: &Path, client_log: &Path) -> Self {
        let logs = Logs::read(&LogSources {
            session_id: None,
            client: client_log,
            core: &crash_dir.join("no-core-log"),
            started_at_ms: 0,
        });
        let mut header = Vec::new();
        if let Ok(file) = fs::File::open(client_log) {
            let _ = file
                .take(section::HEADER_BYTES as u64)
                .read_to_end(&mut header);
        }
        let identified = header
            .split(|byte| *byte == b'\n')
            .any(|line| line.starts_with(section::CLIENT_SESSION_START.as_bytes()));
        let mut folders: Vec<_> = folder::list(crash_dir)
            .into_iter()
            .filter(|folder| {
                folder::read(folder).is_ok_and(|report| {
                    super::main_report(report.kind)
                        && (!identified
                            || (!report.session_id.is_empty()
                                && section::session::matches_header(
                                    &header,
                                    section::CLIENT_SESSION_START,
                                    &report.session_id,
                                )))
                })
            })
            .collect();
        folders.sort_by_cached_key(|folder| super::written_at_ms(folder));
        let mut saved: Vec<_> = folders.iter().filter(|folder| {
            state::read(folder).is_none() &&
                matches!(fs::metadata(folder.join(super::EXIT_FILE)), Err(error) if error.kind() == io::ErrorKind::NotFound)
        }).cloned().collect();
        // Without a session header, the newest saved main-process report is the only candidate
        // that can be associated with the immediately previous launch.
        if !identified && saved.len() > 1 {
            saved.drain(..saved.len() - 1);
        }
        Self {
            logs,
            folders,
            saved,
        }
    }

    /// The most recent previously unconsented main-process report for the legacy relaunch dialog.
    pub fn latest_saved(&self) -> Option<&Path> {
        self.saved.last().map(PathBuf::as_path)
    }

    /// Completes saved reports from the stopped process without ApplicationExitInfo or networking.
    /// Consented reports remain immutable, so a pending retry keeps its original payload.
    pub fn complete_saved(&self) -> io::Result<Vec<PathBuf>> {
        let mut completed = Vec::new();
        for folder in &self.saved {
            if state::read(folder).is_some() {
                continue;
            }
            if !self.logs.client.is_empty() {
                fs::write(folder.join(folder::CLIENT_LOG_FILE), &self.logs.client)?;
            }
            completed.push(folder.clone());
        }
        Ok(completed)
    }
}
