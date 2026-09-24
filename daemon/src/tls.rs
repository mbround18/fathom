//! mTLS for the API listener. The client certificate is mandatory and must chain to the
//! requestheader CA, so a completed handshake IS the "verified front-proxy peer" fact that
//! [`crate::identity::PeerVerified`] carries. Connections that fail the handshake never reach
//! the router.

use crate::identity::PeerVerified;
use anyhow::{Context, Result, bail};
use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rustls::RootCertStore;
use rustls::crypto::ring::default_provider;
use rustls::server::WebPkiClientVerifier;
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub fn server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    client_ca_pem: &[u8],
) -> Result<rustls::ServerConfig> {
    let provider = Arc::new(default_provider());
    let certs: Vec<_> = rustls_pemfile::certs(&mut &*cert_pem)
        .collect::<std::result::Result<_, _>>()
        .context("parsing server certificate")?;
    let key = rustls_pemfile::private_key(&mut &*key_pem)
        .context("parsing server key")?
        .context("no private key found")?;
    let mut roots = RootCertStore::empty();
    for ca in rustls_pemfile::certs(&mut &*client_ca_pem) {
        roots.add(ca.context("parsing requestheader CA")?)?;
    }
    if roots.is_empty() {
        bail!("requestheader CA bundle contains no certificates");
    }
    // Mandatory client auth: no anonymous connections.
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .context("building client verifier")?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

pub async fn serve(
    listener: TcpListener,
    cfg: Arc<rustls::ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
) {
    let acceptor = TlsAcceptor::from(cfg);
    // Handshake succeeded => client cert verified against the requestheader CA.
    let app = app.layer(Extension(PeerVerified(true)));
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => return,
            accepted = listener.accept() => {
                let Ok((tcp, _)) = accepted else { continue };
                let (acceptor, app) = (acceptor.clone(), app.clone());
                tokio::spawn(async move {
                    let tls = match acceptor.accept(tcp).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::debug!(error = %e, "tls handshake rejected");
                            return;
                        }
                    };
                    let svc = TowerToHyperService::new(app);
                    if let Err(e) = Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(tls), svc)
                        .await
                    {
                        tracing::debug!(error = %e, "connection ended");
                    }
                });
            }
        }
    }
}
