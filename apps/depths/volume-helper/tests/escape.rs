//! The escape suite (spec 053 SC-003): every case in `cases::cases()` acts inside the root or is
//! refused, and the sentinels outside the root keep their content, mtime and inode. Concurrent
//! link and folder swaps run against reads and writes, and `/proc` magic links are refused.

mod cases;

use std::io::{Cursor, Read, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use cases::{Access, Case, Expect, SENTINEL_MARK, Tree, build_tree, cases, snapshot};
use depths_volume_helper::resolve::Root;
use depths_volume_helper::{EXIT_OK, serve};
use depths_volumes::frame::{FrameKind, read_frame, write_data, write_frame, write_json};
use depths_volumes::mount::{EntryKind, WriteBlocked};
use depths_volumes::op::{Hello, HelperError, Op, Reply, parse_answer};
use depths_volumes::path::RelPath;
use rustix::fs::{OFlags, RenameFlags};

/// What the resolver did with a case, in the table's terms.
fn outcome(root: &Root, case: &Case) -> Expect {
    let Ok((path, raw)) = case.rel() else {
        return Expect::InvalidPath;
    };
    let result = match case.access {
        Access::Stat => root
            .lookup(&path, raw.as_ref())
            .map(|found| match found.link {
                Some(link) if !link.inside => Expect::OutsideLink,
                _ => Expect::Inside,
            }),
        Access::Read => root.open_read(&path, raw.as_ref()).map(|(mut file, _)| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            assert!(
                !contains(&bytes, SENTINEL_MARK),
                "{}: read a sentinel",
                case.name
            );
            Expect::Inside
        }),
        Access::Write => root.open_write(&path).and_then(|target| {
            let mut file = target.reopen(OFlags::WRONLY | OFlags::TRUNC)?;
            file.write_all(b"written").unwrap();
            Ok(Expect::Inside)
        }),
    };
    match result {
        Ok(expect) => expect,
        Err(HelperError::OutsideMount) => Expect::OutsideMount,
        Err(HelperError::WriteBlocked { reason }) => Expect::Blocked(reason),
        Err(HelperError::NotFound) => Expect::NotFound,
        Err(other) => panic!("{}: unexpected error {other:?}", case.name),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn fresh() -> (tempfile::TempDir, Tree, Root) {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let root = Root::open(&tree.root).unwrap();
    (dir, tree, root)
}

#[test]
fn the_table_has_at_least_thirty_cases() {
    let all = cases();
    assert!(all.len() >= 30, "{} cases", all.len());
    let refused = all.iter().filter(|c| c.expect.is_refused()).count();
    assert!(refused >= 30, "{refused} refused cases");
}

#[test]
fn openat2_is_available() {
    let (_dir, _tree, root) = fresh();
    assert!(root.probe_openat2(), "the escape suite needs openat2");
}

#[test]
fn every_case_stays_inside_or_is_refused() {
    let mut failures = Vec::new();
    for case in cases() {
        // A fresh tree per case: a write inside must not change what the next case sees.
        let (_dir, tree, root) = fresh();
        let before = snapshot(&tree.sentinels);
        let got = outcome(&root, &case);
        if got != case.expect {
            failures.push(format!(
                "{}: expected {:?}, got {got:?}",
                case.name, case.expect
            ));
        }
        assert_eq!(
            snapshot(&tree.sentinels),
            before,
            "{}: a sentinel changed",
            case.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn stat_cases_through_serve_match_the_table() {
    let stat_cases: Vec<Case> = cases()
        .into_iter()
        .filter(|c| c.access == Access::Stat && c.expect != Expect::InvalidPath)
        .collect();
    let (_dir, tree, root) = fresh();
    let before = snapshot(&tree.sentinels);

    let mut input = Vec::new();
    for case in &stat_cases {
        write_json(&mut input, FrameKind::Op, &case.op().unwrap()).unwrap();
    }
    let mut output = Vec::new();
    let code = serve(root.into_fd(), Cursor::new(input), &mut output);
    assert_eq!(code, EXIT_OK);

    let mut out = Cursor::new(output);
    let hello: Hello = read_frame(&mut out)
        .unwrap()
        .unwrap()
        .parse(FrameKind::Hello)
        .unwrap();
    assert!(hello.openat2);
    for case in &stat_cases {
        let answer = parse_answer(&read_frame(&mut out).unwrap().unwrap()).unwrap();
        let got = match answer {
            Ok(Reply::Stat(info)) => match info.link {
                Some(link) if !link.inside => {
                    assert_eq!(info.write_blocked, Some(WriteBlocked::OutsideLink));
                    Expect::OutsideLink
                }
                _ => Expect::Inside,
            },
            Ok(other) => panic!("{}: unexpected reply {other:?}", case.name),
            Err(HelperError::OutsideMount) => Expect::OutsideMount,
            Err(HelperError::NotFound) => Expect::NotFound,
            Err(other) => panic!("{}: unexpected error {other:?}", case.name),
        };
        assert_eq!(got, case.expect, "{}", case.name);
    }
    assert!(read_frame(&mut out).unwrap().is_none());
    assert_eq!(snapshot(&tree.sentinels), before);
}

/// Swaps `root/<a>` and `root/<b>` atomically in a loop until stopped.
fn swapper(root: &std::path::Path, a: &str, b: &str) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let dir: OwnedFd = rustix::fs::open(
        root,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap();
    let (a, b) = (a.to_owned(), b.to_owned());
    let flag = stop.clone();
    let handle = thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            rustix::fs::renameat_with(&dir, a.as_str(), &dir, b.as_str(), RenameFlags::EXCHANGE)
                .unwrap();
        }
    });
    (stop, handle)
}

const RACE_ROUNDS: usize = 3000;

#[test]
fn a_link_swapped_out_during_reads_never_reads_outside() {
    let (_dir, tree, root) = fresh();
    std::os::unix::fs::symlink("sub/b.txt", tree.root.join("swap")).unwrap();
    std::os::unix::fs::symlink("../outside/secret.txt", tree.root.join("swap_alt")).unwrap();
    let before = snapshot(&tree.sentinels);
    let (stop, handle) = swapper(&tree.root, "swap", "swap_alt");
    let path = RelPath::parse("swap").unwrap();
    let (mut inside, mut refused) = (0, 0);
    for _ in 0..RACE_ROUNDS {
        match root.open_read(&path, None) {
            Ok((mut file, _)) => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"hello b");
                inside += 1;
            }
            Err(HelperError::OutsideMount) | Err(HelperError::NotFound) => refused += 1,
            Err(HelperError::OsError { errno, .. }) if errno == "EAGAIN" => refused += 1,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert_eq!(inside + refused, RACE_ROUNDS);
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn a_link_swapped_out_during_writes_never_writes_outside() {
    let (_dir, tree, root) = fresh();
    std::os::unix::fs::symlink("sub/b.txt", tree.root.join("swap")).unwrap();
    std::os::unix::fs::symlink("../outside/secret.txt", tree.root.join("swap_alt")).unwrap();
    let before = snapshot(&tree.sentinels);
    let (stop, handle) = swapper(&tree.root, "swap", "swap_alt");
    let path = RelPath::parse("swap").unwrap();
    for _ in 0..RACE_ROUNDS {
        let result = root
            .open_write(&path)
            .and_then(|target| target.reopen(OFlags::WRONLY | OFlags::TRUNC));
        match result {
            Ok(mut file) => file.write_all(b"written").unwrap(),
            Err(HelperError::OutsideMount) | Err(HelperError::NotFound) => {}
            Err(HelperError::OsError { errno, .. }) if errno == "EAGAIN" => {}
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn a_folder_swapped_for_a_link_mid_walk_never_leaves_the_root() {
    let (_dir, tree, root) = fresh();
    std::fs::create_dir(tree.root.join("walk")).unwrap();
    std::fs::write(tree.root.join("walk/x"), b"inside x").unwrap();
    std::os::unix::fs::symlink("../outside/dir", tree.root.join("walk_alt")).unwrap();
    let before = snapshot(&tree.sentinels);
    let (stop, handle) = swapper(&tree.root, "walk", "walk_alt");
    let path = RelPath::parse("walk/x").unwrap();
    for i in 0..RACE_ROUNDS {
        match i % 3 {
            0 => match root.open_read(&path, None) {
                Ok((mut file, _)) => {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes, b"inside x");
                }
                Err(HelperError::OutsideMount) | Err(HelperError::NotFound) => {}
                Err(HelperError::OsError { errno, .. }) if errno == "EAGAIN" => {}
                Err(other) => panic!("unexpected {other:?}"),
            },
            1 => match root
                .open_write(&path)
                .and_then(|t| t.reopen(OFlags::WRONLY))
            {
                Ok(mut file) => file.write_all(b"inside x").unwrap(),
                Err(HelperError::OutsideMount) | Err(HelperError::NotFound) => {}
                Err(HelperError::OsError { errno, .. }) if errno == "EAGAIN" => {}
                Err(other) => panic!("unexpected {other:?}"),
            },
            _ => match root.lookup(&path, None) {
                Ok(found) => assert_eq!(found.stat.stx_size, 8),
                Err(HelperError::OutsideMount) | Err(HelperError::NotFound) => {}
                Err(other) => panic!("unexpected {other:?}"),
            },
        }
    }
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn proc_magic_links_are_refused() {
    // `/proc/self` as a root: `root`, `cwd`, `exe` and `fd/*` are magic links.
    let root = Root::open("/proc/self").unwrap();
    for path in ["root/etc/hostname", "cwd/Cargo.toml", "exe", "fd/0"] {
        let rel = RelPath::parse(path).unwrap();
        assert!(
            matches!(root.open_read(&rel, None), Err(HelperError::OutsideMount)),
            "{path} was opened"
        );
    }
    let rel = RelPath::parse("root").unwrap();
    let found = root.lookup(&rel, None).unwrap();
    assert!(!found.link.unwrap().inside);
    assert!(matches!(
        root.lookup(&RelPath::parse("root/etc").unwrap(), None),
        Err(HelperError::OutsideMount)
    ));
}

#[test]
fn stat_reports_kinds_and_write_blocks() {
    let (_dir, _tree, root) = fresh();
    let stat = |p: &str| {
        depths_volume_helper::ops::stat(&root, &RelPath::parse(p).unwrap(), None, false).unwrap()
    };
    assert_eq!(stat("").kind, EntryKind::Dir);
    let fifo = stat("fifo");
    assert_eq!(fifo.kind, EntryKind::Special);
    assert_eq!(fifo.write_blocked, Some(WriteBlocked::Special));
    let hard = stat("hard_in");
    assert_eq!(hard.nlink, 2);
    assert_eq!(hard.write_blocked, Some(WriteBlocked::HardLink));
    let link = stat("link_in");
    assert_eq!(link.kind, EntryKind::Link);
    assert!(link.link.as_ref().unwrap().inside);
    assert_eq!(link.size, b"hello b".len() as u64);
    assert_eq!(link.write_blocked, None);
    let out = stat("link_out");
    assert_eq!(out.write_blocked, Some(WriteBlocked::OutsideLink));
    assert_eq!(out.link.unwrap().target, "../outside/secret.txt");

    let hashed =
        depths_volume_helper::ops::stat(&root, &RelPath::parse("a.txt").unwrap(), None, true)
            .unwrap();
    let fp = hashed.fingerprint.unwrap();
    assert_eq!(fp.size(), 7);
    assert_eq!(fp.ino(), hashed.ino);
    assert_eq!(fp.mtime_ns(), hashed.mtime_ns);
}

/// Every name under `dir` that looks like a save's temp.
fn save_temps(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".depths-save-")
        {
            found.push(entry.path());
        }
        if entry.file_type().unwrap().is_dir() {
            found.extend(save_temps(&entry.path()));
        }
    }
    found
}

/// Runs `ops` (each with its Data… End when given) through `serve` and returns the answers;
/// the Data… End that follow a reply are skipped.
fn serve_ops(root: Root, ops: &[(Op, Option<&[u8]>)]) -> Vec<Result<Reply, HelperError>> {
    let mut input = Vec::new();
    for (op, data) in ops {
        write_json(&mut input, FrameKind::Op, op).unwrap();
        if let Some(data) = data {
            write_data(&mut input, data).unwrap();
            write_frame(&mut input, FrameKind::End, &[]).unwrap();
        }
    }
    let mut output = Vec::new();
    assert_eq!(
        serve(root.into_fd(), Cursor::new(input), &mut output),
        EXIT_OK
    );
    let mut out = Cursor::new(output);
    let _: Hello = read_frame(&mut out)
        .unwrap()
        .unwrap()
        .parse(FrameKind::Hello)
        .unwrap();
    let mut answers = Vec::new();
    while let Some(frame) = read_frame(&mut out).unwrap() {
        match frame.kind {
            FrameKind::Reply | FrameKind::Error => answers.push(parse_answer(&frame).unwrap()),
            FrameKind::Data | FrameKind::End => {}
            other => panic!("unexpected {other:?} frame"),
        }
    }
    answers
}

#[test]
fn write_cases_through_serve_save_inside_or_are_refused() {
    let mut failures = Vec::new();
    let write_cases = cases()
        .into_iter()
        .filter(|c| c.access == Access::Write && c.expect != Expect::InvalidPath);
    for case in write_cases {
        let (_dir, tree, root) = fresh();
        let before = snapshot(&tree.sentinels);
        let op = case.op().unwrap();
        let first = serve_ops(root, &[(op.clone(), Some(b"written"))]);
        let got = match &first[0] {
            // The table's base is arbitrary: a file inside answers with its real fingerprint,
            // and the save is then made with it.
            Err(HelperError::ChangedOnDisk { current }) => {
                let Op::Write { path, .. } = op else {
                    unreachable!()
                };
                let retry = Op::Write {
                    path,
                    base: *current,
                    mode: None,
                };
                let root = Root::open(&tree.root).unwrap();
                match &serve_ops(root, &[(retry, Some(b"written"))])[0] {
                    Ok(Reply::Written { .. }) => Expect::Inside,
                    other => panic!("{}: the retry got {other:?}", case.name),
                }
            }
            Err(HelperError::OutsideMount) => Expect::OutsideMount,
            Err(HelperError::WriteBlocked { reason }) => Expect::Blocked(*reason),
            Err(HelperError::NotFound) => Expect::NotFound,
            other => panic!("{}: unexpected answer {other:?}", case.name),
        };
        if got != case.expect {
            failures.push(format!(
                "{}: expected {:?}, got {got:?}",
                case.name, case.expect
            ));
        }
        assert_eq!(
            snapshot(&tree.sentinels),
            before,
            "{}: a sentinel changed",
            case.name
        );
        assert!(
            save_temps(&tree.root).is_empty(),
            "{}: a temp was left",
            case.name
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
