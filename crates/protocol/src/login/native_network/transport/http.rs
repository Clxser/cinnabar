//! Full-ICE HTTP signaling using the original native proof and accepted transport owners.

use super::{Transport, identity::TransportIdentity};
use nethernet_tokio::{ClientIceConfig, ConnectionConfig, signaling::http::HttpSignaling};
use std::{io, net::SocketAddr, sync::Arc, time::Instant};

/// Uses the same actual ready association/Inbox/retirement owner as accepting connections.
pub(in crate::login::native_network) async fn connect(
    address: SocketAddr,
    identity: TransportIdentity,
    deadline: tokio::time::Instant,
) -> io::Result<Transport> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "local HTTP transport requires a numeric loopback endpoint",
        ));
    }
    let cutoff = identity
        .expires
        .map_or(deadline, |expires| deadline.min(expires.into()));
    let result = tokio::time::timeout_at(cutoff, async {
        check_deadline(cutoff, identity.expires)?;
        let mut bind = address;
        bind.set_port(0);
        let signaling =
            HttpSignaling::with_client(rand::random::<u64>().to_string(), client(cutoff)?);
        let client = bedrock_client_tokio::connect::http::connect(
            signaling,
            format!("http://{address}"),
            config(&identity, Some(bind)),
            cutoff,
            identity.expires,
        )
        .await?;
        check_deadline(cutoff, identity.expires)?;
        Ok(Transport::from_http(client))
    })
    .await;
    // Credential expiry keeps its authentication classification rather than becoming a transport timeout.
    check_deadline(cutoff, identity.expires)?;
    result.map_err(|_| timed_out())?
}

/// Uses ordinary verified TLS roots and an explicit provider without changing global crypto state.
pub(super) fn client(deadline: tokio::time::Instant) -> io::Result<reqwest::Client> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(io::Error::other)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(deadline.saturating_duration_since(tokio::time::Instant::now()))
        .build()
        .map_err(io::Error::other)
}

/// Supplies the original proof, trust, incoming bound and explicit ICE binding without dialing.
pub(super) fn config(identity: &TransportIdentity, bind: Option<SocketAddr>) -> ConnectionConfig {
    ConnectionConfig {
        max_incoming_message_bytes: bedrock_net::BatchLimits::default().max_encoded_bytes,
        identity: Some(Arc::clone(&identity.identity)),
        server_trust: Some(identity.trust.clone()),
        client_ice: Some(ClientIceConfig {
            turn_streams: Vec::new(),
            turn_tls: None,
            ice: Default::default(),
            bind_addr: bind,
        }),
        attempts: 1,
        ..Default::default()
    }
}

/// Rechecks original cutoffs even when an awaited result is already ready.
fn check_deadline(cutoff: tokio::time::Instant, expires: Option<Instant>) -> io::Result<()> {
    if let Some(expires) = expires {
        super::super::authentication::ensure_current(expires, Instant::now())?;
    }
    if tokio::time::Instant::now() >= cutoff {
        return Err(timed_out());
    }
    Ok(())
}

/// Omits endpoint/proof details from the finite transport failure.
fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "native HTTP join timed out")
}
