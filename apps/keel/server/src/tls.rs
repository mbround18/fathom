//! HTTPS on the UI, `/api` and `/hooks` listener (contracts/config.md, `--tls-cert/--tls-key`).
//!
//! [`load`] reads the certificate chain and key once, at start, and refuses a missing, unreadable
//! or mismatched pair. [`serve_until`] is [`crate::serve::serve_until`] with a TLS handshake in
//! front of each connection: rustls with the ring provider, TLS 1.2 and 1.3 with rustls'
//! defaults, HTTP/1.1 only (ALPN `http/1.1`). Without TLS configured, the plain listener is used:
//! a TLS proxy in front, or `--insecure-dev`.
//!
//! Errors name the setting and the file, never key material.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::{self, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{self, ServerConfig};
use tower::Service as _;

use crate::config::TlsFiles;
use crate::serve::{DRAIN, HEADER_READ_TIMEOUT};

const CERT: &str = "--tls-cert (KEEL_TLS_CERT)";
const KEY: &str = "--tls-key (KEEL_TLS_KEY)";

/// A PEM error without its contents (a malformed line could be part of a key).
fn pem_reason(e: &pem::Error, what: &str) -> String {
    match e {
        pem::Error::Io(io) => format!("cannot read it: {}", io.kind()),
        pem::Error::NoItemsFound => format!("it holds no PEM {what}"),
        _ => "it is not valid PEM".to_owned(),
    }
}

/// Reads the certificate chain and private key and builds the acceptor.
///
/// # Errors
///
/// Either file is missing, unreadable or not PEM, the chain is empty, or the key does not match
/// the certificate. The message names the setting and the path.
pub fn load(files: &TlsFiles) -> Result<TlsAcceptor, String> {
    let shown = |p: &Path| p.display().to_string();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&files.cert)
        .and_then(Iterator::collect)
        .map_err(|e| {
            format!(
                "{CERT}: {}: {}",
                shown(&files.cert),
                pem_reason(&e, "certificate")
            )
        })?;
    if certs.is_empty() {
        return Err(format!(
            "{CERT}: {}: it holds no PEM certificate",
            shown(&files.cert)
        ));
    }
    let key = PrivateKeyDer::from_pem_file(&files.key).map_err(|e| {
        format!(
            "{KEY}: {}: {}",
            shown(&files.key),
            pem_reason(&e, "private key")
        )
    })?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS: {e}"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| match e {
            rustls::Error::InconsistentKeys(_) => format!(
                "{KEY}: {} does not match the certificate in {}",
                shown(&files.key),
                shown(&files.cert)
            ),
            other => format!("{KEY}: {}: {other}", shown(&files.key)),
        })?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// [`serve_until`], stopping on SIGTERM or Ctrl-C.
///
/// # Errors
///
/// Never at present.
pub async fn serve(listener: TcpListener, tls: TlsAcceptor, app: Router) -> std::io::Result<()> {
    serve_until(listener, tls, app, crate::serve::shutdown_signal()).await
}

/// The HTTPS accept loop. The handshake runs in the connection's task, under the same deadline
/// as reading the request headers, so a slow client never stalls the accept loop.
///
/// # Errors
///
/// Never at present.
pub async fn serve_until(
    listener: TcpListener,
    tls: TlsAcceptor,
    app: Router,
    shutdown: impl Future<Output = ()>,
) -> std::io::Result<()> {
    let graceful = GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let tls = tls.clone();
        let app = app.clone();
        let watcher = graceful.watcher();
        tokio::spawn(async move {
            let stream = match tokio::time::timeout(HEADER_READ_TIMEOUT, tls.accept(stream)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::debug!(%peer, error = %e, "TLS handshake failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(%peer, "TLS handshake timed out");
                    return;
                }
            };
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
                req.extensions_mut().insert(ConnectInfo::<SocketAddr>(peer));
                app.clone().call(req)
            });
            let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            builder
                .http1()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT);
            let conn = builder
                .http1_only()
                .serve_connection_with_upgrades(TokioIo::new(stream), svc)
                .into_owned();
            if let Err(e) = watcher.watch(conn).await {
                tracing::debug!(error = %e, "connection ended with an error");
            }
        });
    }
    drop(listener);
    tracing::info!("shutting down");
    tokio::select! {
        () = graceful.shutdown() => {}
        () = tokio::time::sleep(DRAIN) => tracing::warn!("connections still open after drain"),
    }
    Ok(())
}
