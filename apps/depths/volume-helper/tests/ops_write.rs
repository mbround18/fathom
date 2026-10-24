//! `Write` through `serve` (T039): the save goes through a temp and a rename (R6), keeps the
//! file's uid, gid and mode, checks the base right before the rename, and leaves the old content
//! and no temp behind whenever it does not complete.

mod support;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use depths_volume_helper::EXIT_PROTOCOL;
use depths_volumes::frame::{FrameKind, write_data, write_json};
use depths_volumes::mount::WriteBlocked;
use depths_volumes::op::{HelperError, Op, Reply};
use depths_volumes::text::Fingerprint;
use support::{Script, one, rp};

fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// The file's fingerprint as the helper computes it.
fn fingerprint(root: &Path, path: &str) -> Fingerprint {
    let op = Op::Stat {
        path: rp(path),
        raw_name: None,
        hash: true,
    };
    match one(root, &op, None).0 {
        Ok(Reply::Stat(info)) => info.fingerprint.expect("a file's fingerprint"),
        other => panic!("Stat {path}: {other:?}"),
    }
}

fn write_op(path: &str, base: Fingerprint, mode: Option<u32>) -> Op {
    Op::Write {
        path: rp(path),
        base,
        mode,
    }
}

/// Every name under `dir` that looks like a save's temp.
fn temps(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".depths-save-") {
            found.push(name);
        }
        if entry.file_type().unwrap().is_dir() {
            found.extend(temps(&entry.path()));
        }
    }
    found
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn a_write_keeps_owner_and_mode_and_sends_the_prior_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("conf.yaml");
    std::fs::write(&file, b"old: 1\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    let before = std::fs::metadata(&file).unwrap();
    let base = fingerprint(dir.path(), "conf.yaml");

    let (answer, mut rest) = one(
        dir.path(),
        &write_op("conf.yaml", base, None),
        Some(b"new: 2\n"),
    );
    let Ok(Reply::Written {
        fingerprint: new,
        prior,
    }) = answer
    else {
        panic!("no Written: {answer:?}");
    };
    assert!(prior);
    assert_eq!(rest.data().unwrap(), b"old: 1\n");
    rest.done();

    assert_eq!(std::fs::read(&file).unwrap(), b"new: 2\n");
    let after = std::fs::metadata(&file).unwrap();
    assert_eq!(mode_of(&file), 0o640);
    assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
    assert_ne!(after.ino(), before.ino(), "saved by rename, not in place");
    assert_eq!(new, fingerprint(dir.path(), "conf.yaml"));
    assert!(temps(dir.path()).is_empty());
}

#[test]
fn a_write_with_a_mode_sets_exactly_that_mode() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("run.sh");
    std::fs::write(&file, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let base = fingerprint(dir.path(), "run.sh");

    let (answer, mut rest) = one(
        dir.path(),
        &write_op("run.sh", base, Some(0o750)),
        Some(b"#!/bin/sh\necho hi\n"),
    );
    assert!(matches!(answer, Ok(Reply::Written { prior: true, .. })));
    rest.data().unwrap();
    rest.done();
    assert_eq!(mode_of(&file), 0o750);
}

#[test]
fn an_empty_write_empties_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), b"text").unwrap();
    let base = fingerprint(dir.path(), "a");
    let (answer, mut rest) = one(dir.path(), &write_op("a", base, None), Some(b""));
    let Ok(Reply::Written { fingerprint, .. }) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(fingerprint.size(), 0);
    assert_eq!(rest.data().unwrap(), b"text");
    rest.done();
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"");
}

#[test]
fn a_stale_base_is_refused_with_the_current_fingerprint_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, b"first").unwrap();
    let base = fingerprint(dir.path(), "a.txt");
    std::fs::write(&file, b"someone else").unwrap();
    let current = fingerprint(dir.path(), "a.txt");

    // The data is read and dropped, so the next op is answered.
    let mut answers = Script::new()
        .op(&write_op("a.txt", base, None))
        .data(&vec![b'x'; 600 * 1024])
        .op(&Op::List {
            path: rp(""),
            page: 0,
        })
        .run(dir.path());
    assert_eq!(
        answers.answer(),
        Err(HelperError::ChangedOnDisk { current })
    );
    assert!(matches!(answers.answer(), Ok(Reply::Page { .. })));
    answers.done();
    assert_eq!(std::fs::read(&file).unwrap(), b"someone else");
    assert!(temps(dir.path()).is_empty());
}

#[test]
fn a_stream_cut_mid_way_is_a_protocol_error_and_leaves_the_old_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, b"old").unwrap();
    let base = fingerprint(dir.path(), "a.txt");

    let mut frames = Vec::new();
    write_data(&mut frames, &vec![b'n'; 300 * 1024]).unwrap();
    frames.truncate(frames.len() - 100);
    let answers = Script::new()
        .op(&write_op("a.txt", base, None))
        .raw(&frames)
        .run(dir.path());
    assert_eq!(answers.code, EXIT_PROTOCOL);
    assert_eq!(std::fs::read(&file).unwrap(), b"old");
    assert!(temps(dir.path()).is_empty());
}

#[cfg(debug_assertions)]
#[test]
fn a_helper_killed_mid_stream_leaves_the_old_content_and_no_temp() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, b"old").unwrap();
    let base = fingerprint(dir.path(), "a.txt");

    let mut child = Command::new(env!("CARGO_BIN_EXE_depths-volume-helper"))
        .env("DEPTHS_HELPER_ROOT", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    write_json(&mut stdin, FrameKind::Op, &write_op("a.txt", base, None)).unwrap();
    write_data(&mut stdin, &vec![b'n'; 2 * 1024 * 1024]).unwrap();
    stdin.flush().unwrap();
    // The helper is now blocked reading the next frame, with the bytes so far in its temp.
    std::thread::sleep(std::time::Duration::from_millis(300));
    child.kill().unwrap();
    child.wait().unwrap();
    drop(stdin);

    assert_eq!(std::fs::read(&file).unwrap(), b"old");
    assert!(temps(dir.path()).is_empty());
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(names.len(), 1);
}

#[test]
fn a_parent_the_owner_cannot_write_gives_the_os_error_and_no_change() {
    if is_root() {
        eprintln!("skipped: root writes a 0555 folder anyway");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("a.txt"), b"old").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let base = fingerprint(dir.path(), "locked/a.txt");

    let (answer, rest) = one(
        dir.path(),
        &write_op("locked/a.txt", base, None),
        Some(b"new"),
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    match answer {
        Err(HelperError::OsError { errno, message }) => {
            assert_eq!(errno, "EACCES");
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    rest.done();
    assert_eq!(std::fs::read(locked.join("a.txt")).unwrap(), b"old");
    assert!(temps(dir.path()).is_empty());
}

#[test]
fn prior_is_false_above_ten_mib() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("big.log");
    std::fs::write(&file, vec![b'o'; 10 * 1024 * 1024 + 1]).unwrap();
    let base = fingerprint(dir.path(), "big.log");
    let (answer, rest) = one(dir.path(), &write_op("big.log", base, None), Some(b"short"));
    assert!(matches!(answer, Ok(Reply::Written { prior: false, .. })));
    rest.done();
    assert_eq!(std::fs::read(&file).unwrap(), b"short");
}

#[test]
fn prior_is_sent_at_exactly_ten_mib() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("big.log");
    std::fs::write(&file, vec![b'o'; 10 * 1024 * 1024]).unwrap();
    let base = fingerprint(dir.path(), "big.log");
    let (answer, mut rest) = one(dir.path(), &write_op("big.log", base, None), Some(b"short"));
    assert!(matches!(answer, Ok(Reply::Written { prior: true, .. })));
    assert_eq!(rest.data().unwrap().len(), 10 * 1024 * 1024);
    rest.done();
}

#[test]
fn a_write_through_a_link_inside_replaces_the_target_and_keeps_the_link() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("etc")).unwrap();
    std::fs::write(dir.path().join("etc/real.conf"), b"old").unwrap();
    std::os::unix::fs::symlink("etc/real.conf", dir.path().join("current.conf")).unwrap();
    let base = fingerprint(dir.path(), "etc/real.conf");

    let (answer, mut rest) = one(
        dir.path(),
        &write_op("current.conf", base, None),
        Some(b"new"),
    );
    assert!(matches!(answer, Ok(Reply::Written { prior: true, .. })));
    assert_eq!(rest.data().unwrap(), b"old");
    rest.done();
    assert_eq!(
        std::fs::read(dir.path().join("etc/real.conf")).unwrap(),
        b"new"
    );
    let link = std::fs::symlink_metadata(dir.path().join("current.conf")).unwrap();
    assert!(link.file_type().is_symlink());
    assert!(temps(dir.path()).is_empty());
}

#[test]
fn refusals_drain_the_data_so_the_next_op_is_answered() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), b"shared").unwrap();
    std::fs::hard_link(dir.path().join("a"), dir.path().join("b")).unwrap();
    std::fs::create_dir(dir.path().join("d")).unwrap();
    let base = Fingerprint::of_content(b"shared", 0, 0);

    let mut answers = Script::new()
        .op(&write_op("a", base, None))
        .data(b"new")
        .op(&write_op("missing", base, None))
        .data(b"new")
        .op(&write_op("d", base, None))
        .data(b"new")
        .run(dir.path());
    assert_eq!(
        answers.answer(),
        Err(HelperError::WriteBlocked {
            reason: WriteBlocked::HardLink
        })
    );
    assert_eq!(answers.answer(), Err(HelperError::NotFound));
    assert!(matches!(answers.answer(), Err(HelperError::OsError { .. })));
    answers.done();
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"shared");
}

#[test]
fn a_file_owned_by_someone_else_is_refused_owner_changed() {
    if !is_root() {
        eprintln!("skipped: chown to another uid needs root");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a");
    std::fs::write(&file, b"old").unwrap();
    std::os::unix::fs::chown(&file, Some(4321), Some(4321)).unwrap();
    let base = fingerprint(dir.path(), "a");
    let (answer, rest) = one(dir.path(), &write_op("a", base, None), Some(b"new"));
    assert_eq!(answer, Err(HelperError::OwnerChanged));
    rest.done();
    assert_eq!(std::fs::read(&file).unwrap(), b"old");
}
