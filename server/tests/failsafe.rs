//! Fail-closed startup (spec 031, T043 server part): the built binary exits 2 on invalid
//! configuration and 3 when OIDC discovery fails, and in neither case binds a listener.

use std::net::TcpListener;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_fathom-server");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Run the binary with a clean environment plus `env`; it must exit on its own.
fn run(env: &[(&str, &str)], listen_port: u16) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.env_clear()
        .env("FATHOM_LISTEN", format!("127.0.0.1:{listen_port}"))
        .env("FATHOM_ALLOW_INSECURE_LOCALHOST", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn fathom-server");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("fathom-server kept running instead of failing closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn nothing_listening(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
}

fn base(issuer: &str) -> Vec<(&str, &str)> {
    vec![
        ("FATHOM_EXTERNAL_URL", "http://localhost:8080"),
        ("FATHOM_OIDC_ISSUER", issuer),
        ("FATHOM_OIDC_CLIENT_ID", "fathom"),
    ]
}

#[test]
fn missing_issuer_exits_2_without_listening() {
    let port = free_port();
    let out = run(
        &[
            ("FATHOM_EXTERNAL_URL", "http://localhost:8080"),
            ("FATHOM_OIDC_CLIENT_ID", "fathom"),
        ],
        port,
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(nothing_listening(port));
}

#[test]
fn invalid_issuer_exits_2_without_listening() {
    for bad in ["not a url", "http://idp.example.com/realm"] {
        let port = free_port();
        let out = run(&base(bad), port);
        assert_eq!(out.status.code(), Some(2), "issuer {bad:?}");
        assert!(nothing_listening(port));
    }
}

#[test]
fn unreachable_issuer_exits_3_without_listening() {
    // A loopback port nothing serves: discovery cannot succeed.
    let dead = free_port();
    let issuer = format!("http://127.0.0.1:{dead}/realm");
    let port = free_port();
    let out = run(&base(&issuer), port);
    assert_eq!(out.status.code(), Some(3));
    assert!(nothing_listening(port));
}
