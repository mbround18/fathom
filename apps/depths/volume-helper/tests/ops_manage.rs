//! `Create`, `Rename` and `Delete` through `serve` (T052): a new entry is owned by the helper's
//! uid:gid with the default mode, a rename stays within the mount and never replaces, and a
//! delete removes the entry itself (a link, never its target), a folder only when `recursive`,
//! and says `partial` when a recursive delete stops part way.

mod cases;
mod support;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use cases::{build_tree, snapshot};
use depths_volumes::op::{CreateKind, HelperError, Op, Reply};
use depths_volumes::path::{Name, RawName};
use support::{one, rp};

fn umask() -> u32 {
    // Read, not set: setting the umask would race the other tests' file creation.
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("Umask:"))
        .map(|v| u32::from_str_radix(v.trim(), 8).unwrap())
        .unwrap()
}

fn create(root: &Path, parent: &str, name: &str, kind: CreateKind) -> Result<Reply, HelperError> {
    let op = Op::Create {
        parent: rp(parent),
        name: Name::new(name).unwrap(),
        kind,
    };
    let (answer, rest) = one(root, &op, None);
    rest.done();
    answer
}

fn rename(root: &Path, from: &str, to: &str) -> Result<Reply, HelperError> {
    let op = Op::Rename {
        from: rp(from),
        to: rp(to),
    };
    let (answer, rest) = one(root, &op, None);
    rest.done();
    answer
}

/// The answer, and the prior content when the reply says it follows.
fn delete(
    root: &Path,
    path: &str,
    raw: Option<&[u8]>,
    recursive: bool,
) -> (Result<Reply, HelperError>, Option<Vec<u8>>) {
    let op = Op::Delete {
        path: rp(path),
        raw_name: raw.map(|r| RawName::from_bytes(r).unwrap()),
        recursive,
    };
    let (answer, mut rest) = one(root, &op, None);
    let prior = match &answer {
        Ok(Reply::Deleted { prior: true, .. }) => Some(rest.data().unwrap()),
        _ => None,
    };
    rest.done();
    (answer, prior)
}

fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

#[test]
fn create_makes_a_file_and_a_folder_owned_by_the_helper_with_the_default_mode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("world")).unwrap();
    assert_eq!(
        create(dir.path(), "world", "notes.txt", CreateKind::File),
        Ok(Reply::Created)
    );
    assert_eq!(
        create(dir.path(), "world", "region", CreateKind::Dir),
        Ok(Reply::Created)
    );
    let file = std::fs::symlink_metadata(dir.path().join("world/notes.txt")).unwrap();
    let folder = std::fs::symlink_metadata(dir.path().join("world/region")).unwrap();
    assert!(file.is_file());
    assert_eq!(file.len(), 0);
    assert!(folder.is_dir());
    let mask = umask();
    assert_eq!(file.permissions().mode() & 0o7777, 0o666 & !mask);
    assert_eq!(folder.permissions().mode() & 0o7777, 0o777 & !mask);
    for meta in [file, folder] {
        assert_eq!(meta.uid(), rustix::process::getuid().as_raw());
        assert_eq!(meta.gid(), rustix::process::getgid().as_raw());
    }
}

#[test]
fn create_at_the_root_and_on_a_taken_name() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        create(dir.path(), "", "a", CreateKind::File),
        Ok(Reply::Created)
    );
    assert_eq!(
        create(dir.path(), "", "a", CreateKind::File),
        Err(HelperError::Exists)
    );
    assert_eq!(
        create(dir.path(), "", "a", CreateKind::Dir),
        Err(HelperError::Exists)
    );
    std::os::unix::fs::symlink("nowhere", dir.path().join("dangling")).unwrap();
    // A dangling link is a taken name: never created through.
    assert_eq!(
        create(dir.path(), "", "dangling", CreateKind::File),
        Err(HelperError::Exists)
    );
    assert!(!dir.path().join("nowhere").exists());
    assert_eq!(
        create(dir.path(), "missing", "a", CreateKind::File),
        Err(HelperError::NotFound)
    );
}

#[test]
fn create_through_a_link_out_is_refused_and_writes_nothing_outside() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let before = snapshot(&tree.sentinels);
    assert_eq!(
        create(&tree.root, "link_out_dir", "x2", CreateKind::File),
        Err(HelperError::OutsideMount)
    );
    assert_eq!(
        create(&tree.root, "link_out_dir/dir", "y", CreateKind::Dir),
        Err(HelperError::OutsideMount)
    );
    // A link that stays inside is followed.
    assert_eq!(
        create(&tree.root, "link_in_dir", "made", CreateKind::File),
        Ok(Reply::Created)
    );
    assert!(tree.root.join("sub/made").is_file());
    assert_eq!(snapshot(&tree.sentinels), before);
    let outside = dir.path().join("outside");
    assert!(!outside.join("x2").exists());
    assert!(!outside.join("dir/y").exists());
}

#[test]
fn rename_within_a_folder_and_across_folders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a/inner")).unwrap();
    std::fs::create_dir(dir.path().join("b")).unwrap();
    std::fs::write(dir.path().join("a/f.txt"), b"f").unwrap();
    std::fs::write(dir.path().join("a/inner/g"), b"g").unwrap();
    assert_eq!(rename(dir.path(), "a/f.txt", "a/h.txt"), Ok(Reply::Renamed));
    assert_eq!(
        std::fs::read(dir.path().join("a/h.txt")).unwrap(),
        b"f".to_vec()
    );
    assert!(!dir.path().join("a/f.txt").exists());
    assert_eq!(rename(dir.path(), "a/h.txt", "b/h.txt"), Ok(Reply::Renamed));
    assert!(dir.path().join("b/h.txt").is_file());
    // A folder moves with what is in it.
    assert_eq!(rename(dir.path(), "a/inner", "b/inner"), Ok(Reply::Renamed));
    assert_eq!(
        std::fs::read(dir.path().join("b/inner/g")).unwrap(),
        b"g".to_vec()
    );
    // To the root.
    assert_eq!(rename(dir.path(), "b/h.txt", "top.txt"), Ok(Reply::Renamed));
    assert!(dir.path().join("top.txt").is_file());
}

#[test]
fn rename_never_replaces() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), b"a").unwrap();
    std::fs::write(dir.path().join("b"), b"b").unwrap();
    std::fs::create_dir(dir.path().join("empty")).unwrap();
    std::os::unix::fs::symlink("nowhere", dir.path().join("dangling")).unwrap();
    for to in ["b", "empty", "dangling"] {
        assert_eq!(
            rename(dir.path(), "a", to),
            Err(HelperError::Exists),
            "{to}"
        );
    }
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"a".to_vec());
    assert_eq!(std::fs::read(dir.path().join("b")).unwrap(), b"b".to_vec());
    assert!(dir.path().join("empty").is_dir());
    assert_eq!(rename(dir.path(), "gone", "c"), Err(HelperError::NotFound));
    assert_eq!(
        rename(dir.path(), "a", "missing/c"),
        Err(HelperError::NotFound)
    );
}

#[test]
fn rename_of_or_to_the_root_is_an_invalid_name() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), b"a").unwrap();
    assert_eq!(rename(dir.path(), "", "x"), Err(HelperError::InvalidName));
    assert_eq!(rename(dir.path(), "a", ""), Err(HelperError::InvalidName));
}

#[test]
fn rename_stays_within_the_mount_and_moves_a_link_not_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let before = snapshot(&tree.sentinels);
    assert_eq!(
        rename(&tree.root, "a.txt", "link_out_dir/a.txt"),
        Err(HelperError::OutsideMount)
    );
    assert_eq!(
        rename(&tree.root, "link_out_dir/secret.txt", "stolen.txt"),
        Err(HelperError::OutsideMount)
    );
    assert!(tree.root.join("a.txt").is_file());
    assert!(!tree.root.join("stolen.txt").exists());
    // The link itself moves; it still points where it did.
    assert_eq!(
        rename(&tree.root, "link_out", "sub/moved_link"),
        Ok(Reply::Renamed)
    );
    let moved = tree.root.join("sub/moved_link");
    assert!(moved.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&moved).unwrap(),
        Path::new("../outside/secret.txt")
    );
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn rename_between_folders_of_different_owners_is_mixed_owners() {
    if !is_root() {
        eprintln!("skipped: needs root to give a folder another owner");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("mine")).unwrap();
    std::fs::create_dir(dir.path().join("theirs")).unwrap();
    std::fs::write(dir.path().join("mine/f"), b"f").unwrap();
    std::os::unix::fs::chown(dir.path().join("theirs"), Some(4242), Some(4242)).unwrap();
    assert_eq!(
        rename(dir.path(), "mine/f", "theirs/f"),
        Err(HelperError::MixedOwners)
    );
    assert!(dir.path().join("mine/f").is_file());
    // Both folders someone else's: the helper is not running as their owner.
    std::fs::write(dir.path().join("theirs/g"), b"g").unwrap();
    assert_eq!(
        rename(dir.path(), "theirs/g", "theirs/h"),
        Err(HelperError::OwnerChanged)
    );
    assert_eq!(
        create(dir.path(), "theirs", "n", CreateKind::File),
        Err(HelperError::OwnerChanged)
    );
}

#[test]
fn delete_a_file_sends_its_content_for_history() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ops.json"), b"[\"steve\"]").unwrap();
    let (answer, prior) = delete(dir.path(), "ops.json", None, false);
    assert_eq!(
        answer,
        Ok(Reply::Deleted {
            removed: 1,
            prior: true
        })
    );
    assert_eq!(prior.unwrap(), b"[\"steve\"]".to_vec());
    assert!(!dir.path().join("ops.json").exists());
    assert_eq!(
        delete(dir.path(), "ops.json", None, false).0,
        Err(HelperError::NotFound)
    );
}

#[test]
fn delete_a_file_over_the_prior_cap_sends_no_content() {
    let dir = tempfile::tempdir().unwrap();
    let big = std::fs::File::create(dir.path().join("big.bin")).unwrap();
    big.set_len(depths_volume_helper::ops::PRIOR_MAX + 1)
        .unwrap();
    let (answer, prior) = delete(dir.path(), "big.bin", None, false);
    assert_eq!(
        answer,
        Ok(Reply::Deleted {
            removed: 1,
            prior: false
        })
    );
    assert!(prior.is_none());
    assert!(!dir.path().join("big.bin").exists());
}

#[test]
fn delete_a_link_removes_the_link_and_never_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let before = snapshot(&tree.sentinels);
    for link in [
        "link_out",
        "link_out_dir",
        "link_in_dir",
        "abs_etc",
        "loop1",
    ] {
        let (answer, prior) = delete(&tree.root, link, None, true);
        assert_eq!(
            answer,
            Ok(Reply::Deleted {
                removed: 1,
                prior: false
            }),
            "{link}"
        );
        assert!(prior.is_none());
        assert!(tree.root.join(link).symlink_metadata().is_err(), "{link}");
    }
    assert!(tree.root.join("sub/b.txt").is_file());
    assert_eq!(snapshot(&tree.sentinels), before);
    assert!(dir.path().join("outside/dir/x").is_file());
}

#[test]
fn delete_through_a_link_out_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let before = snapshot(&tree.sentinels);
    for path in ["link_out_dir/secret.txt", "link_out_dir/dir"] {
        assert_eq!(
            delete(&tree.root, path, None, true).0,
            Err(HelperError::OutsideMount),
            "{path}"
        );
    }
    assert_eq!(snapshot(&tree.sentinels), before);
    assert!(dir.path().join("outside/dir/x").is_file());
}

#[test]
fn delete_the_root_is_an_invalid_name() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        delete(dir.path(), "", None, true).0,
        Err(HelperError::InvalidName)
    );
}

#[test]
fn a_folder_needs_recursive_unless_it_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("empty")).unwrap();
    std::fs::create_dir(dir.path().join("full")).unwrap();
    std::fs::write(dir.path().join("full/f"), b"f").unwrap();
    assert_eq!(
        delete(dir.path(), "empty", None, false).0,
        Ok(Reply::Deleted {
            removed: 1,
            prior: false
        })
    );
    match delete(dir.path(), "full", None, false).0 {
        Err(HelperError::OsError { errno, .. }) => assert_eq!(errno, "ENOTEMPTY"),
        other => panic!("{other:?}"),
    }
    assert!(dir.path().join("full/f").is_file());
}

#[test]
fn a_recursive_delete_removes_everything_and_counts_it() {
    let dir = tempfile::tempdir().unwrap();
    let world = dir.path().join("world");
    std::fs::create_dir_all(world.join("region/deep")).unwrap();
    std::fs::write(world.join("level.dat"), b"level").unwrap();
    std::fs::write(world.join("region/r.0.0.mca"), b"r").unwrap();
    std::fs::write(world.join("region/deep/x"), b"x").unwrap();
    std::os::unix::fs::symlink("../level.dat", world.join("region/link")).unwrap();
    std::os::unix::fs::symlink("/etc", world.join("etc")).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        world.join("fifo"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    std::fs::write(dir.path().join("keep"), b"keep").unwrap();
    // world, region, deep, level.dat, r.0.0.mca, x, link, etc, fifo
    assert_eq!(
        delete(dir.path(), "world", None, true).0,
        Ok(Reply::Deleted {
            removed: 9,
            prior: false
        })
    );
    assert!(!world.exists());
    assert!(dir.path().join("keep").is_file());
    assert!(Path::new("/etc/passwd").exists());
}

#[test]
fn a_recursive_delete_that_cannot_finish_is_partial() {
    if is_root() {
        eprintln!("skipped: root removes entries from a read-only folder anyway");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("d");
    std::fs::create_dir_all(d.join("locked")).unwrap();
    std::fs::write(d.join("a"), b"a").unwrap();
    std::fs::write(d.join("locked/f"), b"f").unwrap();
    std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let answer = delete(dir.path(), "d", None, true).0;
    std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
    match answer {
        Err(HelperError::Partial {
            removed,
            failed_at,
            errno,
            message,
        }) => {
            assert_eq!(removed, 1, "d/a went first");
            assert_eq!(failed_at, "d/locked/f");
            assert_eq!(errno, "EACCES");
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert!(!d.join("a").exists());
    assert!(d.join("locked/f").is_file());
}

#[test]
fn a_recursive_delete_that_removes_nothing_is_the_plain_error() {
    if is_root() {
        eprintln!("skipped: root removes entries from a read-only folder anyway");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("d");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("a"), b"a").unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
    let answer = delete(dir.path(), "d", None, true).0;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    match answer {
        Err(HelperError::OsError { errno, .. }) => assert_eq!(errno, "EACCES"),
        other => panic!("{other:?}"),
    }
    assert!(d.join("a").is_file());
}

#[test]
fn delete_by_raw_name() {
    let dir = tempfile::tempdir().unwrap();
    let raw = b"caf\xe9";
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub").join(OsStr::from_bytes(raw)),
        b"latin",
    )
    .unwrap();
    let folder = dir.path().join(OsStr::from_bytes(b"dir\xff"));
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("x"), b"x").unwrap();
    let (answer, prior) = delete(dir.path(), "sub", Some(raw), false);
    assert_eq!(
        answer,
        Ok(Reply::Deleted {
            removed: 1,
            prior: true
        })
    );
    assert_eq!(prior.unwrap(), b"latin".to_vec());
    assert_eq!(
        delete(dir.path(), "", Some(b"dir\xff"), true).0,
        Ok(Reply::Deleted {
            removed: 2,
            prior: false
        })
    );
    assert!(!folder.exists());
    assert!(dir.path().join("sub").is_dir());
}
