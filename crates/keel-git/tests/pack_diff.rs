//! Changed paths and runner packs (spec 055 T069, T070), against a mirror fetched from a local
//! bare repository. `git index-pack` checks every pack the way a host would read it.

use std::path::Path;
use std::process::Command;

use keel_core::ports::CommitId;
use keel_git::{Auth, FetchRequest, Mirror};
use uuid::Uuid;

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

struct Fixture {
    dir: tempfile::TempDir,
    mirror: Mirror,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "--bare", "-q", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        let mirror = Mirror::open(&dir.path().join("data"), Uuid::from_u128(7)).unwrap();
        Self { dir, mirror }
    }

    fn work(&self) -> std::path::PathBuf {
        self.dir.path().join("work")
    }

    fn write(&self, file: &str, content: &str) {
        let path = self.work().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// Commits the work tree, pushes and fetches into the mirror.
    fn commit(&self, msg: &str) -> CommitId {
        let work = self.work();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "--allow-empty", "-m", msg]);
        git(&work, &["push", "-q", "origin", "main"]);
        let url = format!("file://{}", self.dir.path().join("origin.git").display());
        self.mirror
            .fetch(&FetchRequest {
                url: &url,
                branch: "main",
                auth: Auth::None,
                scratch: self.dir.path(),
                allow_local: true,
            })
            .unwrap()
    }

    /// Indexes `pack` in a fresh repository and returns the object ids it holds.
    fn index(&self, pack: &[u8]) -> Vec<String> {
        let check = tempfile::tempdir().unwrap();
        git(check.path(), &["init", "--bare", "-q"]);
        let path = check.path().join("in.pack");
        std::fs::write(&path, pack).unwrap();
        git(check.path(), &["index-pack", "-o", "in.idx", "in.pack"]);
        let out = git(check.path(), &["verify-pack", "-v", "in.idx"]);
        out.lines()
            .filter(|l| l.len() > 40 && l.as_bytes()[40] == b' ')
            .map(|l| l[..40].to_owned())
            .collect()
    }
}

#[test]
fn changed_paths_lists_added_modified_removed_and_renamed_files() {
    let f = Fixture::new();
    f.write("playbooks/web.yaml", "- hosts: all\n");
    f.write("README", "hi\n");
    let first = f.commit("first");
    assert_eq!(
        f.mirror.changed_paths(None, &first).unwrap(),
        ["README", "playbooks/web.yaml"].map(String::from).into()
    );
    f.write("playbooks/web.yaml", "- hosts: web\n");
    std::fs::remove_file(f.work().join("README")).unwrap();
    f.write("roles/db/tasks/main.yaml", "[]\n");
    let second = f.commit("second");
    assert_eq!(
        f.mirror.changed_paths(Some(&first), &second).unwrap(),
        ["README", "playbooks/web.yaml", "roles/db/tasks/main.yaml"]
            .map(String::from)
            .into()
    );
    std::fs::rename(
        f.work().join("roles/db/tasks/main.yaml"),
        f.work().join("roles/db/tasks/other.yaml"),
    )
    .unwrap();
    let third = f.commit("rename");
    assert_eq!(
        f.mirror.changed_paths(Some(&second), &third).unwrap(),
        ["roles/db/tasks/main.yaml", "roles/db/tasks/other.yaml"]
            .map(String::from)
            .into()
    );
    let same = f.commit("empty");
    assert!(
        f.mirror
            .changed_paths(Some(&third), &same)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn full_pack_holds_the_whole_history_and_indexes() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let first = f.commit("first");
    f.write("dir/b.txt", "b\n");
    let second = f.commit("second");
    let pack = f.mirror.build_pack(None, &second).unwrap();
    let ids = f.index(&pack.bytes);
    assert_eq!(ids.len(), pack.objects as usize);
    assert!(ids.contains(&first.0), "{ids:?}");
    assert!(ids.contains(&second.0), "{ids:?}");
    // Two commits, three trees (root twice, dir), two blobs.
    assert_eq!(pack.objects, 7);
}

#[test]
fn incremental_pack_leaves_out_what_the_base_has() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let base = f.commit("first");
    f.write("b.txt", "b\n");
    let target = f.commit("second");
    let pack = f.mirror.build_pack(Some(&base), &target).unwrap();
    let ids = f.index(&pack.bytes);
    assert!(!ids.contains(&base.0));
    assert!(ids.contains(&target.0));
    // The new commit, its root tree and b.txt; a.txt is the base's.
    assert_eq!(pack.objects, 3);
    // Nothing new: an empty pack is still a valid one.
    let none = f.mirror.build_pack(Some(&target), &target).unwrap();
    assert_eq!(none.objects, 0);
    assert_eq!(none.bytes.len(), 12 + 20);
}

#[test]
fn unknown_commits_are_refused() {
    let f = Fixture::new();
    f.write("a.txt", "a\n");
    let c = f.commit("first");
    let missing = CommitId("0".repeat(40).replacen('0', "1", 1));
    assert!(f.mirror.build_pack(None, &missing).is_err());
    assert!(f.mirror.build_pack(Some(&missing), &c).is_err());
    assert!(f.mirror.changed_paths(Some(&missing), &c).is_err());
}
