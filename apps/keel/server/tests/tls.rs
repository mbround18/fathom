//! HTTPS on the UI/API listener (`--tls-cert/--tls-key`, contracts/config.md). No database.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use keel_server::config::TlsFiles;
use keel_server::tls;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};

struct Pair {
    cert_pem: String,
    key_pem: String,
    cert_der: rustls::pki_types::CertificateDer<'static>,
}

fn self_signed() -> Pair {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    Pair {
        cert_pem: ck.cert.pem(),
        key_pem: ck.signing_key.serialize_pem(),
        cert_der: ck.cert.der().clone(),
    }
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn files(dir: &Path, cert: &str, key: &str) -> TlsFiles {
    TlsFiles {
        cert: write(dir, "tls.crt", cert),
        key: write(dir, "tls.key", key),
    }
}

/// `load`'s refusal (the acceptor has no `Debug`).
fn load_err(files: &TlsFiles) -> String {
    match tls::load(files) {
        Ok(_) => panic!("loaded"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn a_client_completes_the_handshake_and_gets_a_response() {
    let dir = tempfile::tempdir().unwrap();
    let pair = self_signed();
    let key = files(dir.path(), &pair.cert_pem, &pair.key_pem);
    // The key may be 0400: it is read once, by the owner.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key.key, std::fs::Permissions::from_mode(0o400)).unwrap();
    }
    let acceptor = tls::load(&key).unwrap_or_else(|e| panic!("{e}"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/", get(|| async { "keel over tls" }));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(tls::serve_until(listener, acceptor, app, async {
        let _ = stop_rx.await;
    }));

    let mut roots = RootCertStore::empty();
    roots.add(pair.cert_der.clone()).unwrap();
    let client =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client));
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut stream = connector
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .expect("TLS handshake");
    let (_, session) = stream.get_ref();
    assert!(matches!(
        session.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3 | rustls::ProtocolVersion::TLSv1_2)
    ));
    assert_eq!(session.alpn_protocol(), None);

    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("keel over tls"), "{response}");

    // Plain HTTP on the HTTPS port gets no HTTP response.
    let mut plain = TcpStream::connect(addr).await.unwrap();
    plain
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = plain.read_to_end(&mut buf).await;
    assert!(!String::from_utf8_lossy(&buf).contains("keel over tls"));

    stop_tx.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[test]
fn a_key_that_does_not_match_the_certificate_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (self_signed(), self_signed());
    let err = load_err(&files(dir.path(), &a.cert_pem, &b.key_pem));
    assert!(
        err.contains("KEEL_TLS_KEY") && err.contains("does not match"),
        "{err}"
    );
}

#[test]
fn a_missing_or_unreadable_file_refuses_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let pair = self_signed();
    let cert = write(dir.path(), "tls.crt", &pair.cert_pem);
    let missing = dir.path().join("absent.key");
    let err = load_err(&TlsFiles {
        cert: cert.clone(),
        key: missing.clone(),
    });
    assert!(
        err.contains("KEEL_TLS_KEY") && err.contains(&missing.display().to_string()),
        "{err}"
    );

    let err = load_err(&TlsFiles {
        cert: dir.path().join("absent.crt"),
        key: write(dir.path(), "tls.key", &pair.key_pem),
    });
    assert!(err.contains("KEEL_TLS_CERT"), "{err}");

    // The key file holds the certificate, not a key.
    let err = load_err(&TlsFiles {
        cert: cert.clone(),
        key: cert,
    });
    assert!(err.contains("KEEL_TLS_KEY"), "{err}");
}

#[test]
fn errors_never_echo_key_material() {
    let dir = tempfile::tempdir().unwrap();
    let pair = self_signed();
    let body: String = pair.key_pem.lines().take(2).collect::<Vec<_>>().join("\n");
    let err = load_err(&files(dir.path(), &pair.cert_pem, &body));
    let secret_line = pair.key_pem.lines().nth(1).unwrap();
    assert!(err.contains("KEEL_TLS_KEY"), "{err}");
    assert!(!err.contains(secret_line), "{err}");
}
