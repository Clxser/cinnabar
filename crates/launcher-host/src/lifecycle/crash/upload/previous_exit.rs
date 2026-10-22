//! Android's account of the previous process's end, read on the next launch: it adds the
//! tombstone's frames to the crash folder the dying process wrote, or writes the folder itself
//! when the process died before it could.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crash_report::{
    folder::{self, LogSources, Logs},
    report::{Identity, Kind, Report},
    stack::{self, tombstone},
};

/// `ApplicationExitInfo` reasons this module names.
const REASON_SIGNALED: i32 = 2;
const REASON_LOW_MEMORY: i32 = 3;
const REASON_CRASH: i32 = 4;
const REASON_CRASH_NATIVE: i32 = 5;
const REASON_ANR: i32 = 6;
/// How far before Android's exit timestamp the dying process's own folder may be stamped.
const MATCH_BEFORE_MS: u64 = 120_000;
/// Folder names have second resolution; allow the rounding.
const MATCH_AFTER_MS: u64 = 2_000;
/// Associates one folder with one historical Android process exit.
const EXIT_FILE: &str = "android-exit.json";

/// The stable fields that identify one Android exit across subsequent launches.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct ExitId {
    timestamp_ms: u64,
    reason: i32,
    status: i32,
}

/// One `ApplicationExitInfo` of the previous main process.
#[derive(Clone, Copy, Debug)]
pub struct Exit<'a> {
    /// Unix milliseconds Android recorded the exit at.
    pub timestamp_ms: u64,
    pub reason: i32,
    pub status: i32,
    pub description: &'a str,
    /// `tombstone.pb` of a native crash on API 31+.
    pub tombstone: Option<&'a [u8]>,
}

/// Returns the crash folder of `exit`: the one the process wrote around its exit, with the
/// tombstone's stack added, or a new one from what Android recorded. `in_app` names Cinnabar's
/// own libraries; `identity` describes this build.
pub fn record(
    crash_dir: &Path,
    client_log: &Path,
    exit: &Exit<'_>,
    identity: &Identity,
    in_app: impl Fn(&str) -> bool,
) -> io::Result<PathBuf> {
    let stack = exit
        .tombstone
        .and_then(|bytes| tombstone::stack(bytes, in_app));
    let earliest = exit.timestamp_ms.saturating_sub(MATCH_BEFORE_MS);
    let latest = exit.timestamp_ms.saturating_add(MATCH_AFTER_MS);
    let id = ExitId {
        timestamp_ms: exit.timestamp_ms,
        reason: exit.reason,
        status: exit.status,
    };
    let written = folder::list(crash_dir).into_iter().rev().find(|folder| {
        match fs::read(folder.join(EXIT_FILE)) {
            Ok(bytes) => serde_json::from_slice::<ExitId>(&bytes).is_ok_and(|saved| saved == id),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                folder::created_at_ms(folder).is_some_and(|at| (earliest..=latest).contains(&at))
            }
            Err(_) => false,
        }
    });
    if let Some(folder) = written {
        if let Some(stack) = &stack
            && stack::read(&folder).is_none()
        {
            stack::write(&folder, stack)?;
        }
        mark_exit(&folder, &id)?;
        return Ok(folder);
    }
    let native = exit.reason == REASON_CRASH_NATIVE;
    let message = match &stack {
        Some(stack) if native && !stack.reason.is_empty() => stack.reason.clone(),
        _ => describe(exit),
    };
    let kind = if native { Kind::Native } else { Kind::Abort };
    let mut report = Report::new(kind, message, identity, exit.timestamp_ms);
    report.native = Some(serde_json::json!({ "exit_code": format!("0x{:X}", exit.status) }));
    let logs = Logs::read(&LogSources {
        session_id: None,
        client: client_log,
        core: &crash_dir.join("no-core-log"),
        started_at_ms: 0,
    });
    let folder = folder::write(crash_dir, &report, &logs, None, exit.timestamp_ms)?;
    if let Some(stack) = &stack {
        stack::write(&folder, stack)?;
    }
    mark_exit(&folder, &id)?;
    Ok(folder)
}

/// Atomically associates a completed folder with its Android exit before it can be queued.
fn mark_exit(folder: &Path, id: &ExitId) -> io::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    serde_json::to_writer(file.as_file_mut(), id)?;
    file.persist(folder.join(EXIT_FILE))
        .map_err(|error| error.error)?;
    Ok(())
}

/// `Android ended the app: <reason> (status N): <description>`.
fn describe(exit: &Exit<'_>) -> String {
    let reason = match exit.reason {
        REASON_SIGNALED => "signal",
        REASON_LOW_MEMORY => "low memory",
        REASON_CRASH => "Java crash",
        REASON_CRASH_NATIVE => "native crash",
        REASON_ANR => "not responding",
        _ => "exit",
    };
    let description = exit.description.lines().next().unwrap_or_default().trim();
    if description.is_empty() {
        format!("Android ended the app: {reason} (status {})", exit.status)
    } else {
        format!(
            "Android ended the app: {reason} (status {}): {description}",
            exit.status
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crash_report::report::Thread;

    const EXIT_MS: u64 = 1_760_443_200_000;

    /// Describes a synthetic Android build without any real session identity.
    fn identity() -> Identity {
        Identity {
            session_id: "android-exit".into(),
            started_at_ms: EXIT_MS,
            release: "0.1.26".into(),
            commit: "abc".into(),
            build_profile: "release".into(),
        }
    }

    /// A one-frame `tombstone.pb`: SIGSEGV on thread 7 in `libcinnabar.so`.
    fn tombstone() -> Vec<u8> {
        /// Appends one length-delimited field to the synthetic protobuf.
        fn field(out: &mut Vec<u8>, number: u8, bytes: &[u8]) {
            out.push(number << 3 | 2);
            out.push(bytes.len() as u8);
            out.extend_from_slice(bytes);
        }
        let mut frame = vec![0x08, 0x40];
        field(&mut frame, 6, b"/data/app/lib/arm64/libcinnabar.so");
        field(&mut frame, 8, b"c0ffee");
        let mut thread = vec![0x08, 7];
        field(&mut thread, 4, &frame);
        let mut entry = vec![0x08, 7];
        field(&mut entry, 2, &thread);
        let mut signal = Vec::new();
        field(&mut signal, 2, b"SIGSEGV");
        let mut tombstone = vec![6 << 3, 7];
        field(&mut tombstone, 10, &signal);
        // Field 16's tag needs two varint bytes.
        tombstone.extend_from_slice(&[0x82, 0x01, entry.len() as u8]);
        tombstone.extend_from_slice(&entry);
        tombstone
    }

    /// Supplies one Android process exit with caller-selected native details.
    fn exit(reason: i32, tombstone: Option<&[u8]>) -> Exit<'_> {
        Exit {
            timestamp_ms: EXIT_MS,
            reason,
            status: 11,
            description: "crash\nsecond line",
            tombstone,
        }
    }

    #[test]
    fn a_tombstone_completes_the_folder_the_dying_process_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = Report::new(Kind::Native, "SIGSEGV".into(), &identity(), EXIT_MS - 1_000);
        report.thread = Thread::default();
        let written =
            folder::write(dir.path(), &report, &Logs::default(), None, EXIT_MS - 1_000).unwrap();
        let bytes = tombstone();
        let folder = record(
            dir.path(),
            &dir.path().join("client.log"),
            &exit(REASON_CRASH_NATIVE, Some(&bytes)),
            &identity(),
            |name| name == "libcinnabar.so",
        )
        .unwrap();
        assert_eq!(folder, written);
        let stack = stack::read(&folder).unwrap();
        assert_eq!(stack.reason, "SIGSEGV");
        assert!(stack.modules[0].in_app);
        assert_eq!(folder::list(dir.path()).len(), 1);
    }

    #[test]
    fn an_exit_without_a_folder_gets_one_from_androids_record() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("client.log");
        std::fs::write(&log, "CLIENT_SESSION_START session=x\nlast words\n").unwrap();
        // An older crash is outside the match window and stays separate.
        folder::write(
            dir.path(),
            &Report::new(Kind::Panic, "old".into(), &identity(), EXIT_MS - 3_600_000),
            &Logs::default(),
            None,
            EXIT_MS - 3_600_000,
        )
        .unwrap();
        let folder = record(
            dir.path(),
            &log,
            &exit(REASON_LOW_MEMORY, None),
            &identity(),
            |_| false,
        )
        .unwrap();
        let report = folder::read(&folder).unwrap();
        assert_eq!(report.kind, Kind::Abort);
        assert_eq!(
            report.message,
            "Android ended the app: low memory (status 11): crash"
        );
        assert_eq!(report.release, "0.1.26");
        let logged = std::fs::read_to_string(folder.join(folder::CLIENT_LOG_FILE)).unwrap();
        assert!(logged.contains("last words"));
        assert_eq!(folder::list(dir.path()).len(), 2);
    }

    #[test]
    fn a_native_exit_without_a_folder_is_named_by_its_signal() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = tombstone();
        let folder = record(
            dir.path(),
            &dir.path().join("client.log"),
            &exit(REASON_CRASH_NATIVE, Some(&bytes)),
            &identity(),
            |_| true,
        )
        .unwrap();
        let report = folder::read(&folder).unwrap();
        assert_eq!(report.kind, Kind::Native);
        assert_eq!(report.message, "SIGSEGV");
        assert!(stack::read(&folder).is_some());
    }
    #[test]
    fn distinct_nearby_android_exits_keep_separate_reports() {
        let dir = tempfile::tempdir().unwrap();
        let first_exit = exit(REASON_CRASH_NATIVE, None);
        let first = record(
            dir.path(),
            &dir.path().join("client.log"),
            &first_exit,
            &identity(),
            |_| true,
        )
        .unwrap();
        super::super::consent(&first, crash_report::upload::state::Consent::User, EXIT_MS).unwrap();
        crash_report::upload::state::record(
            &first,
            &crash_report::upload::state::Outcome::Sent,
            EXIT_MS,
        )
        .unwrap();
        let second_exit = Exit {
            timestamp_ms: EXIT_MS + 10_000,
            ..first_exit
        };
        let second = record(
            dir.path(),
            &dir.path().join("client.log"),
            &second_exit,
            &identity(),
            |_| true,
        )
        .unwrap();
        assert_ne!(
            first, second,
            "a sent report must not consume the next crash"
        );
        assert_eq!(
            record(
                dir.path(),
                &dir.path().join("client.log"),
                &second_exit,
                &identity(),
                |_| true,
            )
            .unwrap(),
            second,
            "the same exit remains idempotent"
        );
        assert_eq!(folder::list(dir.path()).len(), 2);
    }
}
