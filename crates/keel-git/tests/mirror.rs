//! Mirror tests against local bare repositories (spec 055 T040): fetch into
//! `<data_dir>/mirrors/<binding id>.git`, no hooks run, `refs/keel/runs/<id>` pins survive a
//! force-push and `gc`, and ancestry classifies `synced`, `duplicate`, `stale` and
//! `history_rewritten`.
//!
//! The `git` CLI only builds the fixtures (with an isolated configuration and fixed dates, so the
//! commit ids are the same on every run); the code under test is gix.

use std::path::{Path, PathBuf};
use std::process::Command;

use keel_core::ports::{CommitId, GitSource};
use keel_git::{Ancestry, Auth, EntryKind, FetchRequest, Mirror, MirrorError};
use uuid::Uuid;

/// Runs `git` with no user or system configuration and fixed identities and dates.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

/// A source repository: a bare `origin.git` and a working clone `work` that pushes to it.
struct Source {
    _dir: tempfile::TempDir,
    bare: PathBuf,
    work: PathBuf,
}

impl Source {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let bare = dir.path().join("origin.git");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "--bare", "-q", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        Self {
            _dir: dir,
            bare,
            work,
        }
    }

    fn url(&self) -> String {
        format!("file://{}", self.bare.display())
    }

    fn commit(&self, file: &str, content: &str, msg: &str) -> CommitId {
        let path = self.work.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", msg]);
        CommitId(git(&self.work, &["rev-parse", "HEAD"]))
    }

    fn push(&self) {
        git(&self.work, &["push", "-q", "origin", "main"]);
    }

    fn force_push(&self) {
        git(&self.work, &["push", "-q", "--force", "origin", "main"]);
    }
}

fn request<'a>(src: &'a str, scratch: &'a Path) -> FetchRequest<'a> {
    FetchRequest {
        url: src,
        branch: "main",
        auth: Auth::None,
        scratch,
        allow_local: true,
    }
}

#[test]
fn fetches_into_the_binding_mirror() {
    let src = Source::new();
    let first = src.commit(".keel/jobs.yaml", "version: 1\n", "first");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let binding = Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_00b1);
    let mirror = Mirror::open(data.path(), binding).expect("mirror");
    assert_eq!(
        mirror.path(),
        data.path()
            .join("mirrors")
            .join(format!("{binding}.git"))
            .as_path()
    );
    let url = src.url();
    let head = mirror
        .fetch(&request(&url, data.path()))
        .expect("fetch from a local bare repository");
    assert_eq!(head, first);
    assert_eq!(mirror.resolve("main").unwrap(), first);
    assert_eq!(
        mirror.read_file(&head, ".keel/jobs.yaml").unwrap(),
        b"version: 1\n"
    );
    assert_eq!(mirror.entry(&head, ".keel").unwrap(), Some(EntryKind::Dir));
    assert_eq!(mirror.entry(&head, "missing.yaml").unwrap(), None);
    // A mirror is bare: there is no work tree.
    assert!(!mirror.path().join(".keel").exists());
}

#[test]
fn reads_symlinks_without_following_them() {
    let src = Source::new();
    std::fs::create_dir_all(src.work.join("playbooks")).unwrap();
    std::os::unix::fs::symlink("../../etc/passwd", src.work.join("playbooks/escape.yaml")).unwrap();
    let head = src.commit("playbooks/web.yaml", "- hosts: all\n", "links");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(2)).unwrap();
    let url = src.url();
    mirror.fetch(&request(&url, data.path())).unwrap();
    assert_eq!(
        mirror.entry(&head, "playbooks/escape.yaml").unwrap(),
        Some(EntryKind::Symlink("../../etc/passwd".into()))
    );
    assert_eq!(
        mirror.entry(&head, "playbooks/web.yaml").unwrap(),
        Some(EntryKind::File)
    );
}

#[test]
fn no_hooks_run_in_the_mirror() {
    let src = Source::new();
    src.commit("a.txt", "a", "a");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(3)).unwrap();
    let marker = data.path().join("hook-ran");
    // Plant every hook a fetch could plausibly trigger, in the mirror and in the source.
    // (The source's own receive hooks would fire on the fixture's pushes, so only the mirror.)
    for dir in [mirror.path().join("hooks")] {
        std::fs::create_dir_all(&dir).unwrap();
        for hook in [
            "post-checkout",
            "post-merge",
            "post-rewrite",
            "reference-transaction",
            "pre-auto-gc",
            "post-receive",
            "update",
            "pre-receive",
        ] {
            let path = dir.join(hook);
            std::fs::write(
                &path,
                format!("#!/bin/sh\necho {hook} >> {}\n", marker.display()),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let url = src.url();
    mirror.fetch(&request(&url, data.path())).unwrap();
    src.commit("b.txt", "b", "b");
    src.push();
    mirror.fetch(&request(&url, data.path())).unwrap();
    assert!(!marker.exists(), "a hook ran");
}

#[test]
fn pins_survive_force_push_and_gc() {
    let src = Source::new();
    src.commit("a.txt", "a", "a");
    let pinned = src.commit("b.txt", "b", "b");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(4)).unwrap();
    let url = src.url();
    mirror.fetch(&request(&url, data.path())).unwrap();
    let run = Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0099);
    mirror.pin(run, &pinned).expect("pin");
    assert_eq!(mirror.pinned(run).unwrap(), Some(pinned.clone()));

    // Rewrite history: the pinned commit is no longer on the branch.
    git(&src.work, &["reset", "-q", "--hard", "HEAD~1"]);
    let rewritten = src.commit("c.txt", "c", "c");
    src.force_push();
    let head = mirror.fetch(&request(&url, data.path())).unwrap();
    assert_eq!(head, rewritten);

    // Collect garbage aggressively in the mirror; the pin keeps the commit alive.
    git(
        mirror.path(),
        &["reflog", "expire", "--expire=now", "--all"],
    );
    git(mirror.path(), &["gc", "-q", "--prune=now"]);
    assert_eq!(mirror.pinned(run).unwrap(), Some(pinned.clone()));
    assert_eq!(mirror.read_file(&pinned, "b.txt").unwrap(), b"b");

    // Unpinning releases it.
    mirror.unpin(run).unwrap();
    assert_eq!(mirror.pinned(run).unwrap(), None);
}

#[test]
fn ancestry_classifies_each_case() {
    let src = Source::new();
    let a = src.commit("a.txt", "a", "a");
    let b = src.commit("b.txt", "b", "b");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(5)).unwrap();
    let url = src.url();
    let head = mirror.fetch(&request(&url, data.path())).unwrap();
    assert_eq!(head, b);

    assert_eq!(mirror.classify(None, &b).unwrap(), Ancestry::First);
    assert_eq!(
        mirror.classify(Some(&a), &b).unwrap(),
        Ancestry::FastForward
    );
    assert_eq!(mirror.classify(Some(&b), &b).unwrap(), Ancestry::Same);
    assert_eq!(mirror.classify(Some(&b), &a).unwrap(), Ancestry::Behind);

    git(&src.work, &["reset", "-q", "--hard", "HEAD~1"]);
    let c = src.commit("c.txt", "c", "c");
    src.force_push();
    let head = mirror.fetch(&request(&url, data.path())).unwrap();
    assert_eq!(head, c);
    assert_eq!(mirror.classify(Some(&b), &c).unwrap(), Ancestry::Rewritten);

    assert_eq!(Ancestry::First.sync_state(), "synced");
    assert_eq!(Ancestry::FastForward.sync_state(), "synced");
    assert_eq!(Ancestry::Same.sync_state(), "duplicate");
    assert_eq!(Ancestry::Behind.sync_state(), "stale");
    assert_eq!(Ancestry::Rewritten.sync_state(), "history_rewritten");
}

#[test]
fn reports_signed_and_unsigned_commits() {
    let src = Source::new();
    src.commit("a.txt", "a", "a");
    let b = src.commit("b.txt", "b", "b");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(6)).unwrap();
    let url = src.url();
    mirror.fetch(&request(&url, data.path())).unwrap();
    let recent = mirror.recent_commits(&b, 10).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, b);
    assert!(recent.iter().all(|c| !c.signed));
    assert_eq!(recent[0].summary, "b");
}

#[test]
fn refuses_local_urls_unless_allowed_and_unknown_branches() {
    let src = Source::new();
    src.commit("a.txt", "a", "a");
    src.push();
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(7)).unwrap();
    let url = src.url();
    let mut req = request(&url, data.path());
    req.allow_local = false;
    assert!(matches!(
        mirror.fetch(&req),
        Err(MirrorError::UrlRefused(_))
    ));
    let mut req = request(&url, data.path());
    req.branch = "nope";
    let r = mirror.fetch(&req);
    assert!(matches!(r, Err(MirrorError::BranchNotFound(_))), "{r:?}");
    // Credentials in the URL are refused: they would land in logs and on disk.
    let mut req = request("https://user:secret@example.org/r.git", data.path());
    req.allow_local = false;
    match mirror.fetch(&req) {
        Err(MirrorError::UrlRefused(msg)) => assert!(!msg.contains("secret")),
        other => panic!("expected UrlRefused, got {other:?}"),
    }
}

#[test]
fn ssh_requires_a_pinned_known_host() {
    let data = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(data.path(), Uuid::from_u128(8)).unwrap();
    let req = FetchRequest {
        url: "ssh://git@example.org/infra.git",
        branch: "main",
        auth: Auth::SshKey {
            private_key: b"not a key",
            known_host: "",
        },
        scratch: data.path(),
        allow_local: false,
    };
    assert!(matches!(
        mirror.fetch(&req),
        Err(MirrorError::KnownHostRequired)
    ));
}

#[test]
fn ssh_command_pins_the_host_key() {
    let cmd = keel_git::mirror::ssh_command(Path::new("/k/key"), Path::new("/k/known_hosts"))
        .expect("command");
    assert!(cmd.contains("StrictHostKeyChecking=yes"), "{cmd}");
    assert!(cmd.contains("UserKnownHostsFile='/k/known_hosts'"), "{cmd}");
    assert!(cmd.contains("IdentitiesOnly=yes"), "{cmd}");
    assert!(cmd.contains("BatchMode=yes"), "{cmd}");
    assert!(keel_git::mirror::ssh_command(Path::new("/k/it's"), Path::new("/k/h")).is_none());
}
