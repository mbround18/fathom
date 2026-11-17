//! A hostile relay against Keel (spec 067 US2: T025, T026, T059–T062, the SC-002 matrix in
//! contracts/relay.md). The relay is an in-test one that a compromised relay host could be: it
//! opens channels it should not, sends requests and frames it should not, holds streams open and
//! floods. Every attempt fails closed, starts no run and leaves an audit row:
//!
//! - a protocol violation quarantines the relay (`relay.quarantine`), drops the link and every
//!   stream it carries, and the relay is not dialled again;
//! - a limit hit or a runner's own failure is refused and audited (`relay.refused`, coalesced
//!   per relay and reason), and the relay stays in service.
//!
//! russh 0.65 refuses a global request (`tcpip-forward`) and an unknown channel request
//! (`exec`, `shell`, `pty-req`, `env`) on the client side itself, with no handler hook: those
//! are refused (asserted here) but can be neither audited nor quarantined until russh exposes a
//! hook.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod relay_kit;
mod repos;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use keel_exec::client_config::{KeelSshClient, client_config, open_subsystem};
use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{self, SecretString, Subsystem};
use keel_exec::relay::{self, CacheMissReason, Frame};
use keel_server::join;
use keel_server::relays::{RelayState, RelaySupervisor, SupervisorOptions};
use keel_server::runner_listener::{self, RunnerHost, RunnerPath};
use relay_kit::{
    Answered, TestRelay, WAIT, closed, connect_over, connected, harness, key, mint, no_run, opts,
    quarantine_reason, quarantines, refused, register, runner_client, runner_session, wait_live,
};
use repos::Harness;
use russh::client::{self, AuthResult, Handle};
use russh::keys::PrivateKeyWithHashAlg;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use uuid::Uuid;

/// A relay Keel has linked to and that answered `Hello`: the starting point of every attack.
struct Linked {
    h: Harness,
    relay: TestRelay,
    id: Uuid,
    sup: RelaySupervisor,
    side: Answered,
}

async fn linked(test: &str, options: SupervisorOptions) -> Option<Linked> {
    let (h, zone) = harness(test).await?;
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), options);
    sup.sync().await.expect("sync");
    let side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    Some(Linked {
        h,
        relay,
        id,
        sup,
        side,
    })
}

impl Linked {
    /// A carried runner stream, held open, that a quarantine must close.
    async fn held(&self, port: u32) -> russh::Channel<russh::server::Msg> {
        let ch = self
            .side
            .handle
            .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", port)
            .await
            .expect("open runner channel");
        wait_live(&self.h, self.id, |l| l.streams >= 1).await;
        ch
    }

    /// The relay was quarantined for `reason`: the row, one `relay.quarantine` row, the held
    /// stream closed, no run, and no redial.
    async fn quarantined(
        mut self,
        reason: &str,
        held: &mut russh::Channel<russh::server::Msg>,
    ) -> Self {
        let live = wait_live(&self.h, self.id, |l| l.state == RelayState::Quarantined).await;
        assert!(
            live.last_error
                .as_deref()
                .is_some_and(|e| e.starts_with(reason)),
            "{live:?}"
        );
        assert_eq!(
            quarantine_reason(&self.h, self.id).await.as_deref(),
            Some(reason)
        );
        let rows = quarantines(&self.h).await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, reason);
        closed(held).await;
        no_run(&self.h).await;
        self.sup.sync().await.expect("sync");
        self.relay.no_link().await;
        self
    }

    /// Still in service: not quarantined, still connected (a Ping cycle later), no run.
    async fn in_service(&self) {
        assert_eq!(quarantine_reason(&self.h, self.id).await, None);
        assert!(quarantines(&self.h).await.is_empty());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            connected(&self.h, self.id),
            "{:?}",
            self.h.app.relay_live.get(self.id)
        );
        no_run(&self.h).await;
    }

    /// Once every carried stream has ended (its refusal noted), flushes the coalesced
    /// refusals and returns `reason`'s `relay.refused` rows, checking they add up to `want`.
    async fn refused_total(&self, reason: &str, want: u64) -> Vec<serde_json::Value> {
        settle(&self.h, self.id).await;
        self.sup.flush_refusals(true).await;
        let rows = refused(&self.h, reason).await;
        let total: u64 = rows.iter().filter_map(|r| r["count"].as_u64()).sum();
        assert_eq!(total, want, "{reason}: {rows:?}");
        rows
    }

    async fn finish(self) {
        self.sup.shutdown();
        self.h.finish().await;
    }
}

fn raw_frame(body: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(body.len())
        .expect("len")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(body);
    out
}

// ---- protocol violations: quarantine (T059) --------------------------------------------------

#[tokio::test]
async fn a_forwarded_channel_to_anything_but_the_runner_port_quarantines() {
    let Some(l) = linked("hostile_forwarded", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    let opened = l
        .side
        .handle
        .channel_open_forwarded_tcpip("10.0.0.1", 22, "10.0.0.5", 2)
        .await;
    assert!(
        opened.is_err(),
        "Keel accepted forwarded-tcpip to 10.0.0.1:22"
    );
    let l = l.quarantined("forbidden_channel", &mut held).await;
    let rows = quarantines(&l.h).await;
    assert!(
        rows[0].1["detail"]
            .as_str()
            .is_some_and(|d| d.contains("10.0.0.1:22")),
        "{rows:?}"
    );
    l.finish().await;
}

#[tokio::test]
async fn a_direct_tcpip_channel_quarantines() {
    let Some(l) = linked("hostile_direct", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    let opened = l
        .side
        .handle
        .channel_open_direct_tcpip("169.254.169.254", 80, "10.0.0.5", 2)
        .await;
    assert!(opened.is_err());
    l.quarantined("forbidden_channel", &mut held)
        .await
        .finish()
        .await;
}

/// A session channel from the relay is a second control channel from Keel's side.
#[tokio::test]
async fn a_session_channel_quarantines() {
    let Some(l) = linked("hostile_session", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    assert!(l.side.handle.channel_open_session().await.is_err());
    l.quarantined("forbidden_channel", &mut held)
        .await
        .finish()
        .await;
}

#[tokio::test]
async fn an_x11_channel_quarantines() {
    let Some(l) = linked("hostile_x11", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    assert!(
        l.side
            .handle
            .channel_open_x11("10.0.0.5", 6000)
            .await
            .is_err()
    );
    l.quarantined("forbidden_channel", &mut held)
        .await
        .finish()
        .await;
}

/// A channel request the relay has no business sending (one russh lets Keel see).
#[tokio::test]
async fn a_channel_request_quarantines() {
    let Some(l) = linked("hostile_chanreq", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    l.side
        .handle
        .exit_status_request(held.id(), 0)
        .await
        .expect("send");
    l.quarantined("forbidden_request", &mut held)
        .await
        .finish()
        .await;
}

#[tokio::test]
async fn an_unknown_control_frame_quarantines() {
    let Some(l) = linked("hostile_unknown_frame", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    l.side
        .inject
        .send(raw_frame(br#"{"t":"Exec","cmd":"id"}"#))
        .expect("inject");
    l.quarantined("bad_frame", &mut held).await.finish().await;
}

#[tokio::test]
async fn an_oversized_control_frame_quarantines() {
    let Some(l) = linked("hostile_oversized", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    let len = u32::try_from(relay::MAX_CONTROL_FRAME_LEN + 1).expect("len");
    l.side
        .inject
        .send(len.to_be_bytes().to_vec())
        .expect("inject");
    let l = l.quarantined("bad_frame", &mut held).await;
    let rows = quarantines(&l.h).await;
    assert!(
        rows[0].1["detail"]
            .as_str()
            .is_some_and(|d| d.contains("exceeds")),
        "{rows:?}"
    );
    l.finish().await;
}

#[tokio::test]
async fn an_out_of_order_frame_quarantines() {
    let Some(l) = linked("hostile_out_of_order", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    let again = Frame::Hello {
        version: "0.9.0-test".into(),
        relay_key_fingerprint: l.relay.fingerprint(),
        max_streams: 256,
        http_cache: false,
    };
    l.side
        .inject
        .send(relay::encode(&again).expect("hello"))
        .expect("inject");
    l.quarantined("bad_frame", &mut held).await.finish().await;
}

#[tokio::test]
async fn a_pong_answering_no_ping_quarantines() {
    let Some(l) = linked("hostile_pong", opts()).await else {
        return;
    };
    let mut held = l.held(1).await;
    let pong = Frame::Pong {
        seq: 1 << 40,
        version: "0.9.0-test".into(),
        streams: 0,
        uptime_s: 1,
    };
    l.side
        .inject
        .send(relay::encode(&pong).expect("pong"))
        .expect("inject");
    l.quarantined("bad_frame", &mut held).await.finish().await;
}

#[tokio::test]
async fn a_host_key_other_than_the_pinned_one_quarantines() {
    let Some((h, zone)) = harness("hostile_host_key").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let pinned = key().public_key().key_data().clone();
    let id = register(&h, zone, "edge-1", &relay.addr.to_string(), &pinned).await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");
    wait_live(&h, id, |l| l.state == RelayState::Quarantined).await;
    assert_eq!(
        quarantine_reason(&h, id).await.as_deref(),
        Some("host_key_mismatch")
    );
    let rows = quarantines(&h).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    sup.sync().await.expect("sync");
    relay.no_link().await;
    no_run(&h).await;
    sup.shutdown();
    h.finish().await;
}

/// T059: a stream that races a quarantine (or revoke) is refused: Keel re-reads the relay row
/// before serving it.
#[tokio::test]
async fn a_stream_racing_a_quarantine_is_refused() {
    let Some(l) = linked("hostile_race", opts()).await else {
        return;
    };
    // Quarantined in the database, the link not yet told.
    sqlx::query(
        "update relays set quarantined_at = now(), quarantine_reason = 'bad_frame' \
         where id = $1",
    )
    .bind(l.id)
    .execute(&l.h.db.owner)
    .await
    .expect("quarantine");
    let mut ch = l
        .side
        .handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 7)
        .await
        .expect("open");
    closed(&mut ch).await;
    let rows = l.refused_total("relay_unusable", 1).await;
    assert_eq!(rows[0]["detail"], "10.0.0.5:7");
    no_run(&l.h).await;
    l.finish().await;
}

// ---- refused, audited, not a violation -------------------------------------------------------

/// `tcpip-forward` is refused by russh without a hook: no audit row or quarantine is possible
/// (see the module docs), but nothing is forwarded and the link carries on.
#[tokio::test]
async fn tcpip_forward_is_refused() {
    let Some(l) = linked("hostile_tcpip_forward", opts()).await else {
        return;
    };
    assert!(
        l.side
            .handle
            .forward_tcpip("0.0.0.0".into(), 2222)
            .await
            .is_err(),
        "Keel granted tcpip-forward"
    );
    l.in_service().await;
    l.finish().await;
}

/// The matrix's `CacheGet`: answered `CacheMiss` and audited; not a violation.
#[tokio::test]
async fn a_cache_get_is_a_miss_and_audited() {
    let Some(mut l) = linked("hostile_cache_get", opts()).await else {
        return;
    };
    let get = Frame::CacheGet {
        req: 9,
        release: "../../etc".into(),
        path: "passwd".into(),
    };
    l.side
        .inject
        .send(relay::encode(&get).expect("get"))
        .expect("inject");
    let answer = tokio::time::timeout(WAIT, l.side.seen.recv())
        .await
        .expect("no answer")
        .expect("control ended");
    assert!(
        matches!(
            answer,
            Frame::CacheMiss {
                req: 9,
                reason: CacheMissReason::Unavailable
            }
        ),
        "{answer:?}"
    );
    let rows = l.refused_total("cache_get", 1).await;
    assert_eq!(rows[0]["detail"], "../../etc/passwd");
    l.in_service().await;
    l.finish().await;
}

/// T026 and the matrix's "inject bytes before the banner": a stream whose runner never
/// authenticates is the runner's failure: refused and audited, never a quarantine.
#[tokio::test]
async fn a_runner_failing_auth_on_a_carried_stream_does_not_quarantine() {
    let Some(l) = linked("hostile_runner_auth", opts()).await else {
        return;
    };
    // Garbage instead of an SSH banner.
    let ch = l
        .side
        .handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 1)
        .await
        .expect("open");
    let mut garbage = ch.into_stream();
    garbage
        .write_all(b"GET / HTTP/1.1\r\n\r\nnot ssh at all\r\n")
        .await
        .expect("write");
    garbage.shutdown().await.expect("eof");
    // A real handshake, then a certificate from nowhere.
    let mut ssh = connect_over(&l.h, &l.side.handle, 2).await;
    let rogue_ca = key();
    let runner = key();
    let cert = {
        use russh::keys::ssh_key::certificate::{Builder, CertType};
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("now")
            .as_secs();
        let mut b = Builder::new_with_random_nonce(
            &mut rand::rng(),
            runner.public_key().key_data().clone(),
            now - 60,
            now + 3600,
        )
        .expect("builder");
        b.cert_type(CertType::User).expect("type");
        b.valid_principal(Uuid::now_v7().to_string())
            .expect("principal");
        b.sign(&rogue_ca).expect("sign")
    };
    let auth = ssh
        .authenticate_openssh_cert("runner", Arc::new(runner), cert)
        .await
        .expect("auth");
    assert!(!matches!(auth, AuthResult::Success), "a rogue cert passed");
    drop(ssh);
    drop(garbage);

    let rows = l.refused_total("runner_unauthenticated", 2).await;
    assert_eq!(rows.len(), 1, "coalesced: {rows:?}");
    l.in_service().await;
    l.finish().await;
}

/// The matrix's "open `forwarded-tcpip` with no runner behind it and attempt auth": a bare key
/// gets enroll-only, and an enrollment without a valid token is refused and audited.
#[tokio::test]
async fn enrollment_without_a_valid_token_is_refused_and_audited() {
    let Some(l) = linked("hostile_enroll", opts()).await else {
        return;
    };
    let reply = enroll_over(&l, 1, "keel-enroll-v1-not-a-token").await;
    assert!(reply.is_none(), "{reply:?}");
    let rows = l.refused_total("enroll_refused", 1).await;
    assert!(
        rows[0]["detail"]
            .as_str()
            .is_some_and(|d| d.ends_with("_token")),
        "{rows:?}"
    );
    let (hosts,): (i64,) = sqlx::query_as("select count(*) from hosts")
        .fetch_one(&l.h.db.owner)
        .await
        .expect("hosts");
    assert_eq!(hosts, 0);
    l.in_service().await;
    l.finish().await;
}

/// T062: enrollments through a relay are rate-limited per relay id, whatever peer the relay
/// claims; a direct enrollment is not affected.
#[tokio::test]
async fn enrollment_through_a_relay_is_rate_limited_per_relay() {
    let options = SupervisorOptions {
        enroll_per_minute: 1,
        enroll_burst: 1,
        ..opts()
    };
    let Some(l) = linked("hostile_enroll_rate", options).await else {
        return;
    };
    let (_, token) = mint(&l.h, "web-1").await;
    // The burst goes on a bad token; the good one is then over the limit and never read.
    assert!(
        enroll_over(&l, 1, "keel-enroll-v1-not-a-token")
            .await
            .is_none()
    );
    assert!(enroll_over(&l, 2, &token).await.is_none());
    let rows = l.refused_total("enroll_rate", 1).await;
    assert_eq!(rows[0]["detail"], "enroll");

    // The token still works directly.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let direct =
        runner_listener::spawn_with(l.h.app.clone(), RunnerHost::new().expect("host"), listener);
    let ssh = direct_connect(&l.h, addr).await;
    let reply = enroll_on(ssh, &token, "web-1").await;
    assert!(
        matches!(reply, Some(frames::Frame::Enrolled { .. })),
        "{reply:?}"
    );
    direct.abort();
    l.finish().await;
}

/// T062: joins through a relay share one budget keyed on the relay (`join::source_key`), whatever
/// peer it claims. Past `KEEL_JOIN_RATE` they are closed before the database and audited as one
/// coalesced `relay.refused`; a direct join from elsewhere is not affected.
#[tokio::test]
async fn joins_through_a_relay_are_rate_limited_per_relay() {
    let Some(l) = linked("hostile_join_rate", opts()).await else {
        return;
    };
    let rate = l.h.app.cfg.join_rate;
    for port in 1..=rate + 2 {
        let reply = join_over(&l, port).await;
        assert!(reply.is_none(), "{reply:?}");
    }
    let rows = l.refused_total("join_rate", 2).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["detail"], "join");

    let allowed = vec!["debian".to_owned()];
    let direct = join::JoinAttempt {
        token: "keel-join-v1-not-a-token",
        inventory_name: "web-1",
        allowed_profiles: &allowed,
        require_signed: false,
        hostname: "web-1",
        os: "Debian",
        arch: "x86_64",
        runner_version: "0.1.0",
    };
    let peer = RunnerPath::Direct {
        peer: Some("203.0.113.9:4000".parse().expect("peer")),
    };
    let outcome = join::join_key(
        &l.h.app,
        &key().public_key().key_data().clone(),
        &direct,
        &peer,
    )
    .await
    .expect("join");
    assert_ne!(outcome, join::JoinOutcome::Closed("rate_limited"));
    let (hosts,): (i64,) = sqlx::query_as("select count(*) from hosts")
        .fetch_one(&l.h.db.owner)
        .await
        .expect("hosts");
    assert_eq!(hosts, 0);
    l.in_service().await;
    l.finish().await;
}

/// T060: a stream that opens and idles is closed at the auth deadline, audited, and not a
/// violation.
#[tokio::test]
async fn idle_streams_close_at_the_auth_deadline() {
    let options = SupervisorOptions {
        auth_deadline: Duration::from_millis(300),
        ..opts()
    };
    let Some(l) = linked("hostile_idle", options).await else {
        return;
    };
    // Silent from the start …
    let mut silent = l
        .side
        .handle
        .channel_open_forwarded_tcpip("keel-runner", 0, "10.0.0.5", 1)
        .await
        .expect("open");
    // … and a handshake that never authenticates.
    let ssh = connect_over(&l.h, &l.side.handle, 2).await;
    closed(&mut silent).await;
    tokio::time::timeout(WAIT, async {
        while !ssh.is_closed() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("an unauthenticated session outlived the deadline");
    let rows = l.refused_total("auth_deadline", 2).await;
    assert_eq!(rows.len(), 1, "coalesced: {rows:?}");
    wait_live(&l.h, l.id, |s| s.streams == 0).await;
    l.in_service().await;
    l.finish().await;
}

/// T061: ten thousand refusals of one kind in a window are one audit row with `count = 10000`.
#[tokio::test]
async fn ten_thousand_refusals_are_one_audit_row() {
    let options = SupervisorOptions {
        max_streams: 0,
        ..opts()
    };
    let Some(l) = linked("hostile_refusal_flood", options).await else {
        return;
    };
    flood(&l.side.handle, 10_000, 50).await;
    let rows = l.refused_total("max_streams", 10_000).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["count"], 10_000);
    let (all,): (i64,) =
        sqlx::query_as("select count(*) from audit_event where action = 'relay.refused'")
            .fetch_one(&l.h.db.owner)
            .await
            .expect("audit");
    assert_eq!(all, 1);
    l.in_service().await;
    l.finish().await;
}

/// T060 and the matrix's "open 10 000 channels": Keel carries at most its cap, refuses the rest
/// over the rate, stays within a memory bound, and serves another relay's runner and a direct
/// runner meanwhile.
#[tokio::test]
async fn a_flooding_relay_does_not_starve_others() {
    // The rate (20 a second, burst 50) refuses most; the accepted ones idle to the deadline.
    const CAP: u32 = 64;
    let options = SupervisorOptions {
        max_streams: CAP as usize,
        auth_deadline: Duration::from_millis(300),
        ..opts()
    };
    let Some(l) = linked("hostile_flood", options).await else {
        return;
    };
    // A second relay in the same zone, with its own link.
    let zone: (Uuid,) = sqlx::query_as("select id from zones where workspace_id = $1")
        .bind(l.h.workspace)
        .fetch_one(&l.h.db.owner)
        .await
        .expect("zone");
    let mut other = TestRelay::start(&l.h).await;
    let other_id = register(
        &l.h,
        zone.0,
        "edge-2",
        &other.addr.to_string(),
        &other.host_key_data(),
    )
    .await;
    l.sup.sync().await.expect("sync");
    let other_side = other.next_link().await.answer(other.fingerprint());
    wait_live(&l.h, other_id, |s| s.state == RelayState::Connected).await;
    // A direct listener.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let direct =
        runner_listener::spawn_with(l.h.app.clone(), RunnerHost::new().expect("host"), listener);

    let rss_before = rss();
    let peak = Arc::new(AtomicU32::new(0));
    let watch = {
        let (app, id, peak) = (l.h.app.clone(), l.id, Arc::clone(&peak));
        tokio::spawn(async move {
            loop {
                if let Some(s) = app.relay_live.get(id) {
                    peak.fetch_max(s.streams, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
    };
    let handle = l.side.handle.clone();
    let flooding = tokio::spawn(async move { flood(&handle, 10_000, 50).await });

    // Meanwhile: a runner over the other relay attaches, and a direct runner enrolls.
    let (host, _ssh, _stream) = runner_session(&l.h, &other_side.handle, "web-1", 1).await;
    assert_eq!(
        l.h.app.runs.path(host).and_then(|p| p.relay()),
        Some(other_id)
    );
    let (_, token) = mint(&l.h, "web-2").await;
    let reply = enroll_on(direct_connect(&l.h, addr).await, &token, "web-2").await;
    assert!(
        matches!(reply, Some(frames::Frame::Enrolled { .. })),
        "{reply:?}"
    );
    assert!(
        !flooding.is_finished(),
        "the flood ended before the others were served"
    );
    flooding.await.expect("flood");
    watch.abort();

    assert!(
        peak.load(Ordering::Relaxed) <= CAP,
        "Keel carried {} streams, over its cap of {CAP}",
        peak.load(Ordering::Relaxed)
    );
    let grown = rss().saturating_sub(rss_before);
    assert!(
        grown < 256 << 20,
        "Keel grew by {} MiB under the flood",
        grown >> 20
    );
    // Every one of the 10 000 is accounted for, in at most one row per reason.
    settle(&l.h, l.id).await;
    l.sup.flush_refusals(true).await;
    let mut total = 0;
    for reason in [
        "max_streams",
        "stream_rate",
        "auth_deadline",
        "runner_unauthenticated",
    ] {
        let rows = refused(&l.h, reason).await;
        assert!(rows.len() <= 1, "{reason} not coalesced: {rows:?}");
        total += rows.iter().filter_map(|r| r["count"].as_u64()).sum::<u64>();
    }
    assert_eq!(total, 10_000, "refusals for 10 000 streams");
    assert!(!refused(&l.h, "stream_rate").await.is_empty());
    assert!(connected(&l.h, other_id));
    assert!(quarantines(&l.h).await.is_empty());
    direct.abort();
    l.finish().await;
}

// ---- helpers -----------------------------------------------------------------------------------

/// Waits until relay `id` carries no stream: every stream task has ended and noted how.
async fn settle(h: &Harness, id: Uuid) {
    wait_live(h, id, |s| s.streams == 0).await;
}

/// Opens `n` runner channels from `tasks` tasks and waits for Keel to close each one.
async fn flood(handle: &russh::server::Handle, n: u32, tasks: u32) {
    let mut set = JoinSet::new();
    for t in 0..tasks {
        let handle = handle.clone();
        set.spawn(async move {
            for i in 0..n / tasks {
                let port = t * (n / tasks) + i;
                let mut ch = handle
                    .channel_open_forwarded_tcpip("keel-runner", 0, "10.9.0.1", port)
                    .await
                    .expect("open");
                closed(&mut ch).await;
            }
        });
    }
    while let Some(done) = set.join_next().await {
        done.expect("flood task");
    }
}

/// This process's resident set, in bytes.
fn rss() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0)
        * 4096
}

/// A bare key's enrollment over the relay: `Enroll` with `token`, and Keel's answer if any.
async fn enroll_over(l: &Linked, port: u32, token: &str) -> Option<frames::Frame> {
    let ssh = connect_over(&l.h, &l.side.handle, port).await;
    enroll_on(ssh, token, "web-1").await
}

async fn enroll_on(
    mut ssh: Handle<KeelSshClient>,
    token: &str,
    name: &str,
) -> Option<frames::Frame> {
    let bare = PrivateKeyWithHashAlg::new(Arc::new(key()), None);
    let auth = ssh
        .authenticate_publickey("runner", bare)
        .await
        .expect("auth");
    assert!(matches!(auth, AuthResult::Success), "enroll-only refused");
    let mut stream = open_subsystem(&ssh, Subsystem::Enroll).await.ok()?;
    write_frame(
        &mut stream,
        &frames::Frame::Enroll {
            token: SecretString::new(token.to_owned()),
            inventory_name: name.to_owned(),
            allowed_profiles: vec!["debian".to_owned()],
            require_signed: false,
        },
    )
    .await
    .ok()?;
    tokio::time::timeout(WAIT, read_frame(&mut stream))
        .await
        .expect("no reply")
        .ok()
        .flatten()
}

/// A bare key's join over the relay with a token that is not one, and Keel's answer if any.
async fn join_over(l: &Linked, port: u32) -> Option<frames::Frame> {
    let mut ssh = connect_over(&l.h, &l.side.handle, port).await;
    let bare = PrivateKeyWithHashAlg::new(Arc::new(key()), None);
    let auth = ssh
        .authenticate_publickey("runner", bare)
        .await
        .expect("auth");
    assert!(matches!(auth, AuthResult::Success), "join-only refused");
    let mut stream = open_subsystem(&ssh, Subsystem::Join).await.ok()?;
    write_frame(
        &mut stream,
        &frames::Frame::JoinRequest {
            token: SecretString::new("keel-join-v1-not-a-token".to_owned()),
            inventory_name: "web-1".to_owned(),
            allowed_profiles: vec!["debian".to_owned()],
            require_signed: false,
            hostname: "web-1".to_owned(),
            os: "Debian".to_owned(),
            arch: "x86_64".to_owned(),
            runner_version: "0.1.0".to_owned(),
        },
    )
    .await
    .ok()?;
    tokio::time::timeout(WAIT, read_frame(&mut stream))
        .await
        .expect("no reply")
        .ok()
        .flatten()
}

async fn direct_connect(h: &Harness, addr: std::net::SocketAddr) -> Handle<KeelSshClient> {
    let stream = TcpStream::connect(addr).await.expect("connect");
    tokio::time::timeout(
        WAIT,
        client::connect_stream(Arc::new(client_config()), stream, runner_client(h)),
    )
    .await
    .expect("handshake hung")
    .expect("handshake")
}
