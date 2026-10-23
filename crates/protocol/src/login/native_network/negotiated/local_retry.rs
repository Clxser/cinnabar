//! A single keyless retry confined to the launcher's selected loopback HTTP server.

use super::*;
use crate::session_wire::CoreMessage;
use bedrock_client_tokio::PacketIo;
use bedrock_protocol_1_26_51::enums::ConnectionDisconnectFailReason;

/// Requires an account-backed local dial and its actual typed server disconnect.
pub(super) fn qualifies(
    local_account: bool,
    connection: &Connection,
    error: &ProtocolError,
) -> bool {
    local_account
        && error.server_disconnect().is_some()
        && matches!(
            super::super::connection::current_terminal(connection),
            Ok(Some(CoreMessage::Disconnect(disconnect)))
                if crate::disconnect::disconnect_reason(disconnect.reason)
                    == ConnectionDisconnectFailReason::Hostwithoutkeys
        )
}

/// Retires the failed association before one offline Login under the original join cutoff.
/// Account credentials remain with the application for any later transfer or remote join.
pub(super) async fn retry(
    mut connection: Connection,
    target: &NativeConnectionTarget,
    login: &SessionLogin<'_>,
    deadline: tokio::time::Instant,
) -> Result<NegotiatedConnection, ProtocolError> {
    target.local_http_address().map_err(failure)?;
    tokio::time::timeout_at(deadline, connection.finish_close())
        .await
        .map_err(|_| {
            failure(io::Error::new(
                io::ErrorKind::TimedOut,
                "local login retry cleanup timed out",
            ))
        })?
        .map_err(failure)?;
    drop(connection);
    if tokio::time::Instant::now() >= deadline {
        return Err(failure(io::Error::new(
            io::ErrorKind::TimedOut,
            "local login retry exceeded the join deadline",
        )));
    }
    tracing::warn!(
        reason = "server_verification_keys_unavailable",
        "retrying local server login with local identity"
    );
    // The second call has no account handoff, so it cannot retry again.
    Box::pin(super::negotiate(target, login, None, deadline)).await
}
