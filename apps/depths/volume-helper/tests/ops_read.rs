//! `List` and `Read` through `serve` (T029): sorting, paging, non-UTF-8 names, links, write
//! blocks, content up to `max`, and a 50,000-entry folder.

mod cases;
mod support;

use std::time::{Duration, Instant};

use cases::{SENTINEL_MARK, build_tree, snapshot};
use depths_volumes::mount::{Entry, EntryKind, PAGE_SIZE, WriteBlocked};
use depths_volumes::op::{HelperError, Op, Reply};
use depths_volumes::path::RawName;
use depths_volumes::text::Fingerprint;
use support::{Script, one, rp};

fn list(root: &std::path::Path, path: &str, page: u32) -> (Vec<Entry>, bool) {
    let op = Op::List {
        path: rp(path),
        page,
    };
    match one(root, &op, None).0 {
        Ok(Reply::Page {
            entries,
            page: got,
            last,
        }) => {
            assert_eq!(got, page);
            (entries, last)
        }
        other => panic!("List {path:?} page {page}: {other:?}"),
    }
}

fn by_name<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no entry {name}"))
}

#[test]
fn folders_come_first_then_names_case_folded() {
    let dir = tempfile::tempdir().unwrap();
    for d in ["b_dir", "A_dir"] {
        std::fs::create_dir(dir.path().join(d)).unwrap();
    }
    for f in ["c", "B.txt", "a.txt"] {
        std::fs::write(dir.path().join(f), b"x").unwrap();
    }
    let (entries, last) = list(dir.path(), "", 0);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["A_dir", "b_dir", "a.txt", "B.txt", "c"]);
    assert!(last);
    let file = by_name(&entries, "a.txt");
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, 1);
    assert_eq!(file.uid, rustix::process::getuid().as_raw());
    assert_eq!(file.gid, rustix::process::getgid().as_raw());
    assert_eq!(file.nlink, 1);
    assert_eq!(file.write_blocked, None);
    assert!(file.mtime.ends_with('Z'), "{}", file.mtime);
    assert_eq!(by_name(&entries, "A_dir").kind, EntryKind::Dir);
}

#[test]
fn listings_are_paged_at_5000() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..=PAGE_SIZE {
        std::fs::File::create(dir.path().join(format!("f{i:05}"))).unwrap();
    }
    let (first, last) = list(dir.path(), "", 0);
    assert_eq!((first.len(), last), (PAGE_SIZE, false));
    assert_eq!(first[0].name, "f00000");
    let (second, last) = list(dir.path(), "", 1);
    assert_eq!((second.len(), last), (1, true));
    assert_eq!(second[0].name, format!("f{PAGE_SIZE:05}"));
    let (past, last) = list(dir.path(), "", 2);
    assert!(past.is_empty() && last);
}

#[test]
fn an_empty_folder_is_one_empty_last_page() {
    let dir = tempfile::tempdir().unwrap();
    let (entries, last) = list(dir.path(), "", 0);
    assert!(entries.is_empty() && last);
}

#[test]
fn names_that_are_not_utf8_are_listed_lossy_with_their_bytes() {
    let base = tempfile::tempdir().unwrap();
    let tree = build_tree(base.path());
    let (entries, _) = list(&tree.root, "sub", 0);
    let bad = entries
        .iter()
        .find(|e| e.name_b64.is_some())
        .expect("the non-UTF-8 name");
    assert_eq!(bad.name, "bad\u{fffd}.txt");
    assert_eq!(bad.name_b64.as_ref().unwrap().as_bytes(), b"bad\xff.txt");
    assert_eq!(bad.size, b"hello bad".len() as u64);
    assert!(by_name(&entries, "b.txt").name_b64.is_none());
}

#[test]
fn links_special_files_and_hard_links_are_listed_with_their_write_blocks() {
    let base = tempfile::tempdir().unwrap();
    let tree = build_tree(base.path());
    let before = snapshot(&tree.sentinels);
    let (entries, _) = list(&tree.root, "", 0);

    let link_in = by_name(&entries, "link_in");
    assert_eq!(link_in.kind, EntryKind::Link);
    let info = link_in.link.as_ref().unwrap();
    assert_eq!((info.target.as_str(), info.inside), ("sub/b.txt", true));
    assert_eq!(link_in.size, b"hello b".len() as u64);
    assert_eq!(link_in.write_blocked, None);

    for name in ["link_out", "link_out_dir", "abs_in", "abs_out", "link_proc"] {
        let e = by_name(&entries, name);
        assert_eq!(e.kind, EntryKind::Link, "{name}");
        assert!(!e.link.as_ref().unwrap().inside, "{name}");
        assert_eq!(e.write_blocked, Some(WriteBlocked::OutsideLink), "{name}");
    }
    // The size of a link out is the link's own, never its target's.
    let out = by_name(&entries, "link_out");
    assert_eq!(out.size, "../outside/secret.txt".len() as u64);

    let hard = by_name(&entries, "hard_in");
    assert_eq!(hard.nlink, 2);
    assert_eq!(hard.write_blocked, Some(WriteBlocked::HardLink));
    for name in ["fifo", "sock"] {
        let e = by_name(&entries, name);
        assert_eq!(e.kind, EntryKind::Special, "{name}");
        assert_eq!(e.write_blocked, Some(WriteBlocked::Special), "{name}");
    }
    assert_eq!(by_name(&entries, "sub").kind, EntryKind::Dir);
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn listing_through_links_stays_inside() {
    let base = tempfile::tempdir().unwrap();
    let tree = build_tree(base.path());
    let (entries, _) = list(&tree.root, "link_in_dir", 0);
    assert!(entries.iter().any(|e| e.name == "b.txt"));
    let op = |path: &str| Op::List {
        path: rp(path),
        page: 0,
    };
    assert_eq!(
        one(&tree.root, &op("link_out_dir"), None).0,
        Err(HelperError::OutsideMount)
    );
    assert_eq!(
        one(&tree.root, &op("missing"), None).0,
        Err(HelperError::NotFound)
    );
    match one(&tree.root, &op("a.txt"), None).0 {
        Err(HelperError::OsError { errno, .. }) => assert_eq!(errno, "ENOTDIR"),
        other => panic!("{other:?}"),
    }
}

fn read_op(path: &str, max: u64) -> Op {
    Op::Read {
        path: rp(path),
        raw_name: None,
        max,
    }
}

#[test]
fn read_returns_the_fingerprint_and_the_content_up_to_max() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, b"hello world").unwrap();
    std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o640)).unwrap();
    let meta = std::fs::metadata(&file).unwrap();

    let (answer, mut rest) = one(dir.path(), &read_op("a.txt", 11), None);
    let Ok(Reply::Content {
        fingerprint,
        owner,
        write_blocked,
        data,
    }) = answer
    else {
        panic!("{answer:?}");
    };
    use std::os::unix::fs::MetadataExt;
    let expected = Fingerprint::of_content(
        b"hello world",
        meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
        meta.ino(),
    );
    assert_eq!(fingerprint, expected);
    assert_eq!(owner.mode, 0o640);
    assert_eq!(owner.uid, meta.uid());
    assert_eq!(write_blocked, None);
    assert!(data);
    assert_eq!(rest.data().unwrap(), b"hello world");
    rest.done();
}

#[test]
fn read_above_max_sends_the_fingerprint_and_no_data() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big"), vec![b'x'; 100]).unwrap();
    let mut answers = Script::new()
        .op(&read_op("big", 99))
        .op(&read_op("big", 100))
        .run(dir.path());
    let Ok(Reply::Content {
        fingerprint, data, ..
    }) = answers.answer()
    else {
        panic!("no Content");
    };
    assert!(!data);
    assert_eq!(fingerprint.size(), 100);
    // Nothing follows: the next frame is the second op's answer.
    let Ok(Reply::Content { data, .. }) = answers.answer() else {
        panic!("no Content");
    };
    assert!(data);
    assert_eq!(answers.data().unwrap().len(), 100);
    answers.done();
}

#[test]
fn read_streams_a_file_larger_than_one_frame() {
    let dir = tempfile::tempdir().unwrap();
    let content: Vec<u8> = (0..3 * 1024 * 1024 + 17).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.path().join("big.bin"), &content).unwrap();
    let (answer, mut rest) = one(dir.path(), &read_op("big.bin", u64::MAX), None);
    assert!(matches!(answer, Ok(Reply::Content { data: true, .. })));
    assert_eq!(rest.data().unwrap(), content);
    rest.done();
}

#[test]
fn read_refuses_what_it_must_and_never_reads_a_sentinel() {
    let base = tempfile::tempdir().unwrap();
    let tree = build_tree(base.path());
    let before = snapshot(&tree.sentinels);
    for (path, expected) in [
        ("link_out", HelperError::OutsideMount),
        ("abs_in", HelperError::OutsideMount),
        ("missing", HelperError::NotFound),
        (
            "fifo",
            HelperError::WriteBlocked {
                reason: WriteBlocked::Special,
            },
        ),
    ] {
        assert_eq!(
            one(&tree.root, &read_op(path, 1024), None).0,
            Err(expected),
            "{path}"
        );
    }
    // A hard link to an outside file is a file inside: readable, and its writes are off.
    let (answer, mut rest) = one(&tree.root, &read_op("hard_out", 1024), None);
    assert!(matches!(
        answer,
        Ok(Reply::Content {
            write_blocked: Some(WriteBlocked::HardLink),
            ..
        })
    ));
    let bytes = rest.data().unwrap();
    assert!(
        !bytes
            .windows(SENTINEL_MARK.len())
            .any(|w| w == SENTINEL_MARK)
    );
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn read_by_raw_name() {
    let base = tempfile::tempdir().unwrap();
    let tree = build_tree(base.path());
    let op = Op::Read {
        path: rp("sub"),
        raw_name: Some(RawName::from_bytes(b"bad\xff.txt".to_vec()).unwrap()),
        max: 1024,
    };
    let (answer, mut rest) = one(&tree.root, &op, None);
    assert!(matches!(answer, Ok(Reply::Content { data: true, .. })));
    assert_eq!(rest.data().unwrap(), b"hello bad");
}

/// SC-002: a 50,000-entry folder lists, every page, in under 1 s. Asserted in release builds;
/// a debug build measures and prints it, as an unoptimised sort is not what ships.
#[test]
fn a_50000_entry_folder_lists_in_under_a_second() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..50_000 {
        if i % 10 == 0 {
            std::fs::create_dir(dir.path().join(format!("d{i}"))).unwrap();
        } else {
            std::fs::File::create(dir.path().join(format!("f{i}"))).unwrap();
        }
    }
    let pages = 50_000u32.div_ceil(PAGE_SIZE as u32);
    let mut script = Script::new();
    for page in 0..pages {
        script = script.op(&Op::List { path: rp(""), page });
    }
    let started = Instant::now();
    let mut answers = script.run(dir.path());
    let took = started.elapsed();

    let mut total = 0;
    for page in 0..pages {
        let Ok(Reply::Page { entries, last, .. }) = answers.answer() else {
            panic!("page {page}");
        };
        assert_eq!(last, page + 1 == pages);
        if page == 0 {
            assert_eq!(entries[0].kind, EntryKind::Dir);
        }
        total += entries.len();
    }
    answers.done();
    assert_eq!(total, 50_000);
    eprintln!("50,000 entries, {pages} pages: {took:?}");
    if !cfg!(debug_assertions) {
        assert!(took < Duration::from_secs(1), "took {took:?}");
    }
}
