//! T040 (spec 067 US5): typed host queries on the runner.
//!
//! - `queries.toml` is parsed strictly (`deny_unknown_fields`, known kinds, valid patterns).
//! - A missing or invalid file refuses every query `not_allowed`.
//! - Units match exactly or by a trailing `*` on a literal prefix, and must match
//!   `^[A-Za-z0-9@._:-]{1,256}$`.
//! - Paths match exactly or below an allowed path; they are absolute, normalised, without `..`,
//!   at most 4096 bytes, and a symlink cannot lead a query outside.
//! - Answers over 64 KiB are refused `too_large`; a fifth query in flight is refused `busy`.
//! - `systemctl` is an argv array with `--` before the unit, and `smartctl` likewise.
//! - Every query lands in the host's query log with who asked and the outcome, never the answer.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use keel_exec::codec::{read_frame, write_frame};
use keel_exec::frames::{Base, Frame, MAX_QUERIES_IN_FLIGHT, QueryRefusal, SmartDisk};
use keel_runner::executor::smart;
use keel_runner::runner::queries::{
    self, Answerer, Checked, InFlight, QUERY_LOG_MAX, QueryConfig, QueryLog, QueryTrust,
};
use tokio::net::UnixListener;

const ALL_KINDS: &str =
    r#"kinds = ["facts", "applied_revision", "unit_state", "disk_health", "path_free"]"#;

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn trust(text: &str) -> QueryTrust {
    QueryTrust::parse(text).unwrap()
}

struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("trust")).unwrap();
        std::fs::create_dir_all(dir.path().join("state")).unwrap();
        Self { dir }
    }

    fn path(&self, p: &str) -> PathBuf {
        self.dir.path().join(p)
    }

    fn write_trust(&self, text: &str) {
        std::fs::write(self.path("trust/queries.toml"), text).unwrap();
    }

    /// An executable script standing in for a host tool.
    fn tool(&self, name: &str, body: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn config(&self) -> QueryConfig {
        let mut cfg = QueryConfig::new(
            self.path("trust"),
            self.path("executor.sock"),
            self.path("venv"),
        );
        cfg.systemctl = self.path("no-systemctl");
        cfg.proc_mounts = self.path("mounts");
        cfg.sys_root = self.path("sys");
        cfg
    }

    fn answerer(&self, cfg: QueryConfig) -> Answerer {
        Answerer::new(cfg, QueryLog::new(self.path("state/queries.log")))
    }

    fn log_lines(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.path("state/queries.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn query(kind: &str, p: &[(&str, &str)]) -> Frame {
    Frame::Query {
        query_id: "q-1".into(),
        kind: kind.into(),
        params: params(p),
        asked_by: "ada".into(),
    }
}

fn refusal(frame: Frame) -> QueryRefusal {
    match frame {
        Frame::QueryRefused { query_id, reason } => {
            assert_eq!(query_id, "q-1");
            reason
        }
        other => panic!("expected QueryRefused, got {other:?}"),
    }
}

fn answer(frame: Frame) -> serde_json::Value {
    match frame {
        Frame::QueryAnswer { query_id, answer } => {
            assert_eq!(query_id, "q-1");
            answer
        }
        other => panic!("expected QueryAnswer, got {other:?}"),
    }
}

// ---- queries.toml ----

#[test]
fn queries_toml_is_parsed_strictly() {
    let t = trust(&format!(
        "{ALL_KINDS}\nunits = [\"k3s.service\", \"longhorn-*\"]\npaths = [\"/var/lib/longhorn\", \"/\"]\n"
    ));
    assert!(t.allows_kind(queries::Kind::Facts));
    // Every key is optional; an empty file answers nothing.
    let empty = trust("");
    assert!(queries::Kind::ALL.iter().all(|k| !empty.allows_kind(*k)));

    for bad in [
        "kinds = [\"facts\"]\nshell = true\n",
        "kinds = [\"exec\"]\n",
        "kinds = \"facts\"\n",
        "units = [\"*\"]\n",
        "units = [\"a*b\"]\n",
        "units = [\"k3s service\"]\n",
        "units = [\"k3s;rm\"]\n",
        "paths = [\"var/lib\"]\n",
        "paths = [\"/var/../etc\"]\n",
        "paths = [\"/var/./lib\"]\n",
        "paths = [\"/var//lib\"]\n",
        "paths = [\"/var/lib/\"]\n",
    ] {
        assert!(QueryTrust::parse(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn a_missing_or_invalid_file_refuses_every_query() {
    let host = Host::new();
    let a = host.answerer(host.config());
    for (kind, p) in [
        ("facts", vec![]),
        ("applied_revision", vec![]),
        ("unit_state", vec![("unit", "k3s.service")]),
        ("disk_health", vec![]),
        ("path_free", vec![("path", "/")]),
    ] {
        let reply = a.answer(query(kind, &p)).await.unwrap();
        assert_eq!(refusal(reply), QueryRefusal::NotAllowed, "{kind}");
    }
    host.write_trust(&format!("{ALL_KINDS}\nunknown = 1\n"));
    let reply = a.answer(query("facts", &[])).await.unwrap();
    assert_eq!(refusal(reply), QueryRefusal::NotAllowed);
    assert!(queries::check(None, "facts", &params(&[])) == Err(QueryRefusal::NotAllowed));
}

#[test]
fn kinds_and_params_are_checked_before_anything_runs() {
    let t = trust(
        r#"kinds = ["facts", "unit_state"]
units = ["k3s.service"]"#,
    );
    let check = |kind: &str, p: &[(&str, &str)]| queries::check(Some(&t), kind, &params(p));
    assert_eq!(check("facts", &[]), Ok(Checked::Facts));
    // Not listed, or not a kind at all.
    assert_eq!(check("disk_health", &[]), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("shell", &[]), Err(QueryRefusal::NotAllowed));
    // Wrong, extra or missing parameters.
    assert_eq!(
        check("facts", &[("unit", "x")]),
        Err(QueryRefusal::BadParams)
    );
    assert_eq!(check("unit_state", &[]), Err(QueryRefusal::BadParams));
    assert_eq!(
        check("unit_state", &[("path", "/")]),
        Err(QueryRefusal::BadParams)
    );
    assert_eq!(
        check("unit_state", &[("unit", "k3s.service"), ("path", "/")]),
        Err(QueryRefusal::BadParams)
    );
}

// ---- units ----

#[test]
fn units_match_exactly_or_by_a_literal_prefix() {
    let t = trust(&format!(
        "{ALL_KINDS}\nunits = [\"k3s.service\", \"longhorn-*\", \"getty@tty1.service\"]\n"
    ));
    let check = |u: &str| queries::check(Some(&t), "unit_state", &params(&[("unit", u)]));
    assert_eq!(
        check("k3s.service"),
        Ok(Checked::UnitState("k3s.service".into()))
    );
    assert!(check("longhorn-manager.service").is_ok());
    assert!(check("getty@tty1.service").is_ok());
    // Not a prefix match on an exact entry, and the prefix is literal.
    assert_eq!(check("k3s.service2"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("k3s"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("longhorn"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("sshd.service"), Err(QueryRefusal::NotAllowed));
    // Not a unit name at all: a shell would care, systemctl never sees it.
    for bad in [
        "",
        "longhorn-$(id)",
        "longhorn-a b",
        "longhorn-a;id",
        "longhorn-*",
        "longhorn-/etc/passwd",
        "longhorn-\n",
        &format!("longhorn-{}", "a".repeat(256)),
    ] {
        assert_eq!(check(bad), Err(QueryRefusal::BadParams), "{bad:?}");
    }
    assert!(queries::valid_unit(&"a".repeat(256)));
    assert!(!queries::valid_unit(&"a".repeat(257)));
    assert!(queries::valid_unit("-flag-looking.service"));
}

#[test]
fn systemctl_is_an_argv_array_with_a_double_dash_before_the_unit() {
    let cmd = queries::unit_state_command(Path::new("/usr/bin/systemctl"), "-H.service");
    assert_eq!(cmd.get_program(), OsStr::new("/usr/bin/systemctl"));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        [
            OsStr::new("show"),
            OsStr::new("--property=LoadState,ActiveState,SubState,ActiveEnterTimestamp"),
            OsStr::new("--"),
            OsStr::new("-H.service"),
        ]
    );
    // An empty environment but for locale and pager settings.
    let envs: Vec<_> = cmd.get_envs().map(|(k, _)| k.to_owned()).collect();
    assert!(
        envs.iter()
            .all(|k| ["LANG", "SYSTEMD_PAGER", "SYSTEMD_COLORS"].contains(&k.to_str().unwrap()))
    );
}

#[tokio::test]
async fn unit_state_answers_from_systemctl_show() {
    let host = Host::new();
    host.write_trust(&format!("{ALL_KINDS}\nunits = [\"k3s.service\"]\n"));
    let args_file = host.path("args");
    let mut cfg = host.config();
    cfg.systemctl = host.tool(
        "systemctl",
        &format!(
            "printf '%s\\n' \"$@\" > {}\nprintf 'LoadState=loaded\\nActiveState=active\\nSubState=running\\nActiveEnterTimestamp=Mon 2026-11-16 10:00:00 UTC\\n'",
            args_file.display()
        ),
    );
    let a = host.answerer(cfg);
    let got = answer(
        a.answer(query("unit_state", &[("unit", "k3s.service")]))
            .await
            .unwrap(),
    );
    assert_eq!(
        got,
        serde_json::json!({"load": "loaded", "active": "active", "sub": "running", "since": "Mon 2026-11-16 10:00:00 UTC"})
    );
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert_eq!(
        args,
        "show\n--property=LoadState,ActiveState,SubState,ActiveEnterTimestamp\n--\nk3s.service\n"
    );
}

#[tokio::test]
async fn an_answer_over_64_kib_is_refused_too_large() {
    let host = Host::new();
    host.write_trust(&format!("{ALL_KINDS}\nunits = [\"big.service\"]\n"));
    let mut cfg = host.config();
    cfg.systemctl = host.tool(
        "systemctl",
        "printf 'LoadState='; head -c 70000 /dev/zero | tr '\\0' a; printf '\\n'",
    );
    let a = host.answerer(cfg);
    let reply = a
        .answer(query("unit_state", &[("unit", "big.service")]))
        .await
        .unwrap();
    assert_eq!(refusal(reply), QueryRefusal::TooLarge);
    assert_eq!(host.log_lines()[0]["outcome"], "too_large");
}

#[tokio::test]
async fn a_missing_systemctl_is_unavailable() {
    let host = Host::new();
    host.write_trust(&format!("{ALL_KINDS}\nunits = [\"k3s.service\"]\n"));
    let a = host.answerer(host.config());
    let reply = a
        .answer(query("unit_state", &[("unit", "k3s.service")]))
        .await
        .unwrap();
    assert_eq!(refusal(reply), QueryRefusal::Unavailable);
}

// ---- paths ----

#[test]
fn paths_match_exactly_or_below_an_allowed_one() {
    let t = trust(&format!(
        "{ALL_KINDS}\npaths = [\"/var/lib/longhorn\", \"/srv\"]\n"
    ));
    let check = |p: &str| queries::check(Some(&t), "path_free", &params(&[("path", p)]));
    assert!(check("/var/lib/longhorn").is_ok());
    assert!(check("/var/lib/longhorn/replicas/a").is_ok());
    assert!(check("/srv").is_ok());
    // A sibling sharing a prefix, a parent, and the root are not below.
    assert_eq!(check("/var/lib/longhorn2"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("/var/lib"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("/"), Err(QueryRefusal::NotAllowed));
    assert_eq!(check("/etc/shadow"), Err(QueryRefusal::NotAllowed));
    // Not absolute or not normalised: refused before matching.
    for bad in [
        "",
        "srv",
        "./srv",
        "/srv/../etc/shadow",
        "/var/lib/longhorn/..",
        "/srv/./x",
        "/srv//x",
        "/srv/",
        "/srv/\0x",
        &format!("/srv/{}", "a".repeat(4096)),
    ] {
        assert_eq!(check(bad), Err(QueryRefusal::BadParams), "{bad:?}");
    }
    assert!(queries::valid_path(&format!("/{}", "a".repeat(4095))));
    assert!(!queries::valid_path(&format!("/{}", "a".repeat(4096))));
    // `/` allows everything below it.
    let root = trust(&format!("{ALL_KINDS}\npaths = [\"/\"]\n"));
    assert!(root.allows_path("/etc"));
}

#[tokio::test]
async fn path_free_answers_statvfs_and_a_symlink_cannot_lead_outside() {
    let host = Host::new();
    let allowed = host.path("data");
    std::fs::create_dir_all(allowed.join("inner")).unwrap();
    std::fs::create_dir_all(host.path("secret")).unwrap();
    std::os::unix::fs::symlink(host.path("secret"), allowed.join("escape")).unwrap();
    std::os::unix::fs::symlink(allowed.join("inner"), allowed.join("stay")).unwrap();
    host.write_trust(&format!(
        "{ALL_KINDS}\npaths = [\"{}\"]\n",
        allowed.display()
    ));
    let a = host.answerer(host.config());

    let path = allowed.join("inner").display().to_string();
    let got = answer(
        a.answer(query("path_free", &[("path", &path)]))
            .await
            .unwrap(),
    );
    for key in ["total_bytes", "free_bytes", "avail_bytes", "inodes_free"] {
        assert!(got[key].is_u64(), "{key}: {got}");
    }
    assert!(got["total_bytes"].as_u64() >= got["free_bytes"].as_u64());

    let stay = allowed.join("stay").display().to_string();
    assert!(matches!(
        a.answer(query("path_free", &[("path", &stay)]))
            .await
            .unwrap(),
        Frame::QueryAnswer { .. }
    ));
    let escape = allowed.join("escape").display().to_string();
    let reply = a
        .answer(query("path_free", &[("path", &escape)]))
        .await
        .unwrap();
    assert_eq!(refusal(reply), QueryRefusal::NotAllowed);
    let missing = allowed.join("missing").display().to_string();
    let reply = a
        .answer(query("path_free", &[("path", &missing)]))
        .await
        .unwrap();
    assert_eq!(refusal(reply), QueryRefusal::Unavailable);
}

// ---- limits ----

#[test]
fn a_fifth_query_in_flight_is_refused_busy() {
    let in_flight = InFlight::default();
    let held: Vec<_> = (0..MAX_QUERIES_IN_FLIGHT)
        .map(|_| in_flight.try_start().expect("a free slot"))
        .collect();
    assert_eq!(held.len(), 4);
    assert!(in_flight.try_start().is_none(), "the fifth is busy");
    drop(held);
    assert!(in_flight.try_start().is_some(), "slots free up");

    let host = Host::new();
    let a = host.answerer(host.config());
    let busy = a.busy(&query("facts", &[])).unwrap();
    assert_eq!(refusal(busy), QueryRefusal::Busy);
    assert_eq!(host.log_lines()[0]["outcome"], "busy");
}

// ---- the other kinds ----

#[tokio::test]
async fn facts_and_applied_revision() {
    let host = Host::new();
    host.write_trust(ALL_KINDS);
    std::fs::write(host.path("uptime"), "12345.67 99.0\n").unwrap();
    std::fs::write(
        host.path("os-release"),
        "NAME=x\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n",
    )
    .unwrap();
    let mut cfg = host.config();
    cfg.proc_uptime = host.path("uptime");
    cfg.os_release = host.path("os-release");

    // A fake executor answering ReadLedger.
    let listener = UnixListener::bind(host.path("executor.sock")).unwrap();
    let exec = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        assert_eq!(read_frame(&mut s).await.unwrap(), Some(Frame::ReadLedger));
        write_frame(
            &mut s,
            &Frame::Ledger {
                ledger_head: Some("ab".repeat(32)),
                bases: vec![Base {
                    job: "site".into(),
                    commit: "c".repeat(40),
                }],
            },
        )
        .await
        .unwrap();
    });
    let a = host.answerer(cfg);
    let facts = answer(a.answer(query("facts", &[])).await.unwrap());
    assert_eq!(facts["uptime_s"], 12345);
    assert_eq!(facts["os"], "Debian GNU/Linux 13 (trixie)");
    assert_eq!(facts["arch"], std::env::consts::ARCH);
    assert_eq!(facts["runner_version"], env!("CARGO_PKG_VERSION"));
    for key in ["hostname", "kernel", "ansible_core"] {
        assert!(facts.get(key).is_some(), "{key}");
    }

    let rev = answer(a.answer(query("applied_revision", &[])).await.unwrap());
    assert_eq!(rev["ledger_head"], "ab".repeat(32));
    assert_eq!(rev["bases"][0]["job"], "site");
    exec.await.unwrap();

    // No executor: unavailable.
    std::fs::remove_file(host.path("executor.sock")).unwrap();
    let reply = a.answer(query("applied_revision", &[])).await.unwrap();
    assert_eq!(refusal(reply), QueryRefusal::Unavailable);
}

#[tokio::test]
async fn disk_health_reads_mounts_and_sys_and_asks_the_executor_for_smart() {
    let host = Host::new();
    host.write_trust(ALL_KINDS);
    std::fs::write(
        host.path("mounts"),
        "sysfs /sys sysfs rw,nosuid 0 0\n\
         /dev/sda1 / ext4 rw,relatime 0 0\n\
         /dev/sdb1 /srv xfs ro,relatime 0 0\n\
         tmpfs /run tmpfs rw 0 0\n",
    )
    .unwrap();
    std::fs::create_dir_all(host.path("sys/fs/ext4/sda1")).unwrap();
    std::fs::write(host.path("sys/fs/ext4/sda1/errors_count"), "2\n").unwrap();
    let listener = UnixListener::bind(host.path("executor.sock")).unwrap();
    let exec = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        assert_eq!(read_frame(&mut s).await.unwrap(), Some(Frame::ReadSmart));
        write_frame(
            &mut s,
            &Frame::Smart {
                installed: true,
                disks: vec![SmartDisk {
                    name: "sda".into(),
                    smart_passed: Some(true),
                }],
            },
        )
        .await
        .unwrap();
    });
    let a = host.answerer(host.config());
    let got = answer(a.answer(query("disk_health", &[])).await.unwrap());
    exec.await.unwrap();
    assert_eq!(
        got["mounts"],
        serde_json::json!([
            {"mount": "/", "device": "/dev/sda1", "fs": "ext4", "read_only": false, "errors": 2},
            {"mount": "/srv", "device": "/dev/sdb1", "fs": "xfs", "read_only": true, "errors": null},
        ])
    );
    assert_eq!(got["smartctl"], true);
    assert_eq!(
        got["disks"],
        serde_json::json!([{"name": "sda", "smart_passed": true}])
    );
}

// ---- the executor's SMART ----

#[test]
fn smartctl_is_an_argv_array_on_disks_the_executor_picked() {
    let cmd = smart::smartctl_command(Path::new("/usr/sbin/smartctl"), "nvme0n1");
    let args: Vec<&OsStr> = cmd.get_args().collect();
    assert_eq!(
        args,
        [
            OsStr::new("--json"),
            OsStr::new("-H"),
            OsStr::new("--"),
            OsStr::new("/dev/nvme0n1"),
        ]
    );
    for ok in ["sda", "nvme0n1", "vda", "mmcblk0"] {
        assert!(smart::is_disk_name(ok), "{ok}");
    }
    for bad in [
        "", "loop0", "dm-0", "zram0", "md127", "sr0", "../sda", "sda/..", "a b",
    ] {
        assert!(!smart::is_disk_name(bad), "{bad}");
    }
    assert_eq!(
        smart::parse_passed(br#"{"smart_status":{"passed":false}}"#),
        Some(false)
    );
    assert_eq!(smart::parse_passed(b"not json"), None);
}

#[tokio::test]
async fn read_smart_runs_smartctl_per_disk_and_says_when_it_is_missing() {
    let host = Host::new();
    for d in ["sda", "loop0", "nvme0n1"] {
        std::fs::create_dir_all(host.path("block").join(d)).unwrap();
    }
    let smartctl = host.tool(
        "smartctl",
        r#"case "$4" in /dev/sda) echo '{"smart_status":{"passed":true}}';; *) echo '{}';; esac; exit 4"#,
    );
    let got = smart::read_smart(&host.path("block"), Some(smartctl)).await;
    assert_eq!(
        got,
        Frame::Smart {
            installed: true,
            disks: vec![
                SmartDisk {
                    name: "nvme0n1".into(),
                    smart_passed: None
                },
                SmartDisk {
                    name: "sda".into(),
                    smart_passed: Some(true)
                },
            ],
        }
    );
    assert_eq!(
        smart::read_smart(&host.path("block"), None).await,
        Frame::Smart {
            installed: false,
            disks: vec![]
        }
    );
}

// ---- the host's query log ----

#[tokio::test]
async fn every_query_is_logged_with_who_asked_and_never_the_answer() {
    let host = Host::new();
    host.write_trust(r#"kinds = ["facts"]"#);
    let a = host.answerer(host.config());
    let facts = answer(a.answer(query("facts", &[])).await.unwrap());
    let reply = a
        .answer(query("unit_state", &[("unit", "sshd.service")]))
        .await
        .unwrap();
    assert_eq!(refusal(reply), QueryRefusal::NotAllowed);

    let lines = host.log_lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["query_id"], "q-1");
    assert_eq!(lines[0]["kind"], "facts");
    assert_eq!(lines[0]["asked_by"], "ada");
    assert_eq!(lines[0]["outcome"], "answered");
    assert!(lines[0]["at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(lines[1]["params"]["unit"], "sshd.service");
    assert_eq!(lines[1]["outcome"], "not_allowed");
    let text = std::fs::read_to_string(host.path("state/queries.log")).unwrap();
    assert!(!text.contains("runner_version"), "no answer in the log");
    assert!(!text.contains(facts["arch"].as_str().unwrap()));
    let mode = std::fs::metadata(host.path("state/queries.log"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "the log is private");
}

#[test]
fn the_log_rotates_at_one_mib_keeping_three() {
    let host = Host::new();
    let log = QueryLog::new(host.path("state/queries.log"));
    let big = "x".repeat(4000);
    let p = params(&[("path", big.as_str())]);
    let per_line = 4200;
    let lines = (QUERY_LOG_MAX as usize / per_line + 2) * 5;
    for _ in 0..lines {
        log.append("q", "path_free", &p, "ada", "not_allowed");
    }
    for n in 1..=3 {
        assert!(host.path(&format!("state/queries.log.{n}")).exists(), "{n}");
    }
    assert!(!host.path("state/queries.log.4").exists());
    let size = std::fs::metadata(host.path("state/queries.log"))
        .unwrap()
        .len();
    assert!(size < QUERY_LOG_MAX + 8192);
}

#[test]
fn keel_supplied_strings_are_cut_in_the_log() {
    let host = Host::new();
    let log = QueryLog::new(host.path("state/queries.log"));
    log.append(
        &"i".repeat(10_000),
        "facts",
        &params(&[]),
        &"a".repeat(10_000),
        "busy",
    );
    let line = &host.log_lines()[0];
    assert_eq!(line["query_id"].as_str().unwrap().len(), 64);
    assert_eq!(line["asked_by"].as_str().unwrap().len(), 256);
}

#[test]
fn rfc3339_formats_unix_seconds() {
    assert_eq!(queries::rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(queries::rfc3339(1_795_000_000), "2026-11-18T11:06:40Z");
    assert_eq!(queries::rfc3339(951_782_400), "2000-02-29T00:00:00Z");
}
