//! Android sends only from the healthy next launch, using the desktop payload and sender.

use jni::{
    JNIEnv,
    objects::{JByteArray, JObject, JString},
    sys::{jboolean, jint, jlong, jstring},
};
use launcher_host::lifecycle::crash::upload::{
    self,
    previous_exit::{self, Exit},
};
use std::path::PathBuf;

/// Records Android's previous exit, optionally consents to it, and retries due reports.
/// A zero timestamp just retries previously consented reports. Java calls on a worker thread.
#[allow(clippy::too_many_arguments, reason = "JNI fixes the native method ABI")]
pub(super) extern "system" fn report<'local>(
    mut env: JNIEnv<'local>,
    _activity: JObject<'local>,
    files: JString<'local>,
    libraries: JString<'local>,
    timestamp: jlong,
    reason: jint,
    status: jint,
    description: JString<'local>,
    trace: JByteArray<'local>,
    user: jboolean,
) -> jstring {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> anyhow::Result<String> {
            let files = PathBuf::from(String::from(env.get_string(&files)?));
            let libraries = PathBuf::from(String::from(env.get_string(&libraries)?));
            let layout = super::layout(&files, &libraries);
            let Some(policy) = upload::Policy::load(&layout) else {
                return Ok("Crash uploads are disabled".into());
            };
            if timestamp > 0 {
                let description = String::from(env.get_string(&description)?);
                let trace = if trace.is_null() {
                    Vec::new()
                } else {
                    env.convert_byte_array(&trace)?
                };
                let identity = crash_report::report::Identity {
                    session_id: String::new(),
                    started_at_ms: timestamp as u64,
                    release: env!("CARGO_PKG_VERSION").into(),
                    commit: option_env!("CINNABAR_BUILD_COMMIT")
                        .unwrap_or("unknown")
                        .into(),
                    build_profile: if cfg!(debug_assertions) {
                        "debug"
                    } else {
                        "release"
                    }
                    .into(),
                };
                let exit = Exit {
                    timestamp_ms: timestamp as u64,
                    reason,
                    status,
                    description: &description,
                    tombstone: (!trace.is_empty()).then_some(trace.as_slice()),
                };
                let folder = previous_exit::record(
                    &layout.crash_dir(),
                    &layout.log_dir().join("client.log"),
                    &exit,
                    &identity,
                    |name| name == format!("lib{}.so", env!("CARGO_CRATE_NAME")),
                )?;
                if user != 0 {
                    upload::consent(
                        &folder,
                        crash_report::upload::state::Consent::User,
                        upload::now_ms(),
                    )?;
                }
                for (folder, result) in upload::send_due(&layout, &policy) {
                    upload::log_result(&folder, &result);
                }
                return Ok(match crash_report::upload::state::read(&folder) {
                    Some(crash_report::upload::state::Upload::Sent { .. }) => "Report sent",
                    Some(crash_report::upload::state::Upload::Pending { .. }) => {
                        "Report queued; will retry on a later launch"
                    }
                    Some(crash_report::upload::state::Upload::Abandoned { .. }) => {
                        "Report could not be sent"
                    }
                    None => "Report saved on this device",
                }
                .into());
            }
            for (folder, result) in upload::send_due(&layout, &policy) {
                upload::log_result(&folder, &result);
            }
            Ok("".into())
        },
    ));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    let message = match outcome {
        Ok(Ok(message)) => message,
        _ => "Could not prepare crash report".into(),
    };
    env.new_string(message)
        .map_or(std::ptr::null_mut(), JString::into_raw)
}
