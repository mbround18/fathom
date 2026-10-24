//! `Size`, `Tar` and `Put` through `serve` (T046): a tar holds the same paths and bytes as the
//! disk and never a link's target, a size walk stops at its cap, and an upload is atomic, refuses
//! a taken name without a matching `replace`, and keeps a replaced file's owner and mode.

mod cases;
mod support;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use cases::{SENTINEL_MARK, build_tree, snapshot};
use depths_volume_helper::EXIT_PROTOCOL;
use depths_volumes::frame::{FrameKind, write_frame};
use depths_volumes::op::{HelperError, Op, Reply, SizeCap};
use depths_volumes::path::{Name, RawName};
use depths_volumes::text::Fingerprint;
use support::{Script, one, rp};

/// What one archive entry or disk entry is, by path.
#[derive(Debug, PartialEq, Eq)]
enum Node {
    Dir,
    File(Vec<u8>),
    Link(Vec<u8>),
}

fn tar_of(root: &Path, path: &str, raw: Option<&[u8]>) -> Result<Vec<u8>, HelperError> {
    let op = Op::Tar {
        path: rp(path),
        raw_name: raw.map(|r| RawName::from_bytes(r).unwrap()),
    };
    let (answer, mut rest) = one(root, &op, None);
    assert_eq!(answer?, Reply::Tar);
    let bytes = rest.data();
    rest.done();
    bytes
}

fn unpack(bytes: &[u8]) -> BTreeMap<Vec<u8>, Node> {
    let mut nodes = BTreeMap::new();
    let mut archive = tar::Archive::new(bytes);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path_bytes().into_owned();
        let path = path.strip_suffix(b"/").unwrap_or(&path).to_vec();
        let node = match entry.header().entry_type() {
            tar::EntryType::Directory => Node::Dir,
            tar::EntryType::Symlink => Node::Link(entry.link_name_bytes().unwrap().into_owned()),
            tar::EntryType::Regular => {
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                Node::File(content)
            }
            other => panic!("unexpected {other:?} entry"),
        };
        assert!(nodes.insert(path, node).is_none(), "a path twice");
    }
    nodes
}

/// The disk under `dir`, named beneath `prefix`, devices, FIFOs and sockets left out.
fn walk(dir: &Path, prefix: &[u8], nodes: &mut BTreeMap<Vec<u8>, Node>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let path = if prefix.is_empty() {
            name.as_bytes().to_vec()
        } else {
            [prefix, b"/", name.as_bytes()].concat()
        };
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() {
            let target = std::fs::read_link(entry.path()).unwrap();
            nodes.insert(path, Node::Link(target.as_os_str().as_bytes().to_vec()));
        } else if kind.is_dir() {
            nodes.insert(path.clone(), Node::Dir);
            walk(&entry.path(), &path, nodes);
        } else if kind.is_file() {
            nodes.insert(path, Node::File(std::fs::read(entry.path()).unwrap()));
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn a_tar_of_the_root_holds_what_the_disk_holds_and_no_link_target() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    std::fs::write(
        tree.root.join("big.bin"),
        (0..700_000u32).map(|i| i as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    std::fs::create_dir(tree.root.join("empty")).unwrap();
    let before = snapshot(&tree.sentinels);

    let bytes = tar_of(&tree.root, "", None).unwrap();
    assert!(!contains(&bytes, SENTINEL_MARK), "a link was followed out");
    let mut disk = BTreeMap::new();
    walk(&tree.root, b"", &mut disk);
    assert_eq!(unpack(&bytes), disk);
    assert!(!disk.contains_key(b"fifo".as_slice()));
    assert_eq!(snapshot(&tree.sentinels), before);
}

#[test]
fn a_tar_of_a_folder_is_named_beneath_it_with_modes_and_owners() {
    let dir = tempfile::tempdir().unwrap();
    let saves = dir.path().join("world/saves");
    std::fs::create_dir_all(saves.join("region")).unwrap();
    std::fs::write(saves.join("level.dat"), b"level").unwrap();
    std::fs::set_permissions(
        saves.join("level.dat"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let long = "n".repeat(150);
    std::fs::write(saves.join("region").join(&long), b"long name").unwrap();

    let bytes = tar_of(dir.path(), "world/saves", None).unwrap();
    let nodes = unpack(&bytes);
    let keys: Vec<String> = nodes
        .keys()
        .map(|k| String::from_utf8_lossy(k).into_owned())
        .collect();
    assert_eq!(
        keys,
        [
            "saves".to_owned(),
            "saves/level.dat".to_owned(),
            "saves/region".to_owned(),
            format!("saves/region/{long}"),
        ]
    );

    let meta = std::fs::metadata(saves.join("level.dat")).unwrap();
    let mut archive = tar::Archive::new(bytes.as_slice());
    let level = archive
        .entries()
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.path_bytes().as_ref() == b"saves/level.dat")
        .unwrap();
    let header = level.header();
    assert_eq!(header.mode().unwrap() & 0o7777, 0o640);
    assert_eq!(header.uid().unwrap(), u64::from(meta.uid()));
    assert_eq!(header.gid().unwrap(), u64::from(meta.gid()));
    assert_eq!(header.mtime().unwrap(), meta.mtime() as u64);
}

#[test]
fn a_tar_by_raw_name_uses_the_raw_name() {
    let dir = tempfile::tempdir().unwrap();
    let raw = b"caf\xe9";
    let folder = dir.path().join(OsStr::from_bytes(raw));
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("x"), b"x").unwrap();
    let nodes = unpack(&tar_of(dir.path(), "", Some(raw)).unwrap());
    assert_eq!(
        nodes.get(b"caf\xe9/x".as_slice()),
        Some(&Node::File(b"x".to_vec()))
    );
}

#[test]
fn a_tar_is_refused_for_a_file_a_missing_path_and_a_link_out() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    assert!(matches!(
        tar_of(&tree.root, "a.txt", None),
        Err(HelperError::OsError { errno, .. }) if errno == "ENOTDIR"
    ));
    assert_eq!(
        tar_of(&tree.root, "missing", None),
        Err(HelperError::NotFound)
    );
    assert_eq!(
        tar_of(&tree.root, "link_out_dir", None),
        Err(HelperError::OutsideMount)
    );
    // A link inside to a folder is followed for the start only.
    let nodes = unpack(&tar_of(&tree.root, "link_in_dir", None).unwrap());
    assert!(nodes.contains_key(b"link_in_dir/b.txt".as_slice()));
}

#[test]
fn an_unreadable_file_ends_the_tar_with_an_error_frame() {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root reads a 0000 file anyway");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("d")).unwrap();
    std::fs::write(dir.path().join("d/a"), b"a").unwrap();
    std::fs::write(dir.path().join("d/b"), b"b").unwrap();
    std::fs::set_permissions(
        dir.path().join("d/b"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    match tar_of(dir.path(), "d", None) {
        Err(HelperError::OsError { errno, .. }) => assert_eq!(errno, "EACCES"),
        other => panic!("{other:?}"),
    }
}

fn size_of(root: &Path, path: &str, bytes: u64, entries: u64) -> Reply {
    let op = Op::Size {
        path: rp(path),
        raw_name: None,
        cap: SizeCap { bytes, entries },
    };
    let (answer, rest) = one(root, &op, None);
    rest.done();
    answer.unwrap()
}

#[test]
fn size_counts_files_and_entries_and_stops_at_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("d/e")).unwrap();
    std::fs::write(dir.path().join("d/a"), vec![0; 1000]).unwrap();
    std::fs::write(dir.path().join("d/e/b"), vec![0; 24]).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.path().join("d/out")).unwrap();

    assert_eq!(
        size_of(dir.path(), "d", u64::MAX, u64::MAX),
        Reply::Size {
            bytes: 1024,
            entries: 4,
            capped: false
        }
    );
    let Reply::Size {
        entries, capped, ..
    } = size_of(dir.path(), "d", u64::MAX, 2)
    else {
        unreachable!()
    };
    assert!(capped);
    assert_eq!(entries, 3, "stops at the first entry past the cap");
    let Reply::Size { bytes, capped, .. } = size_of(dir.path(), "d", 999, u64::MAX) else {
        unreachable!()
    };
    assert!(capped);
    assert!(bytes > 999);
    assert_eq!(
        size_of(dir.path(), "d/a", u64::MAX, u64::MAX),
        Reply::Size {
            bytes: 1000,
            entries: 1,
            capped: false
        }
    );
}

#[test]
fn size_of_the_root_and_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    assert!(matches!(
        size_of(&tree.root, "", u64::MAX, u64::MAX),
        Reply::Size { capped: false, .. }
    ));
    let op = Op::Size {
        path: rp("link_out_dir"),
        raw_name: None,
        cap: SizeCap {
            bytes: 1,
            entries: 1,
        },
    };
    assert_eq!(one(&tree.root, &op, None).0, Err(HelperError::OutsideMount));
}

fn put(dir: &str, name: &str, replace: Option<Fingerprint>) -> Op {
    Op::Put {
        dir: rp(dir),
        name: Name::new(name).unwrap(),
        replace,
    }
}

fn fingerprint(root: &Path, path: &str) -> Fingerprint {
    let op = Op::Stat {
        path: rp(path),
        raw_name: None,
        hash: true,
    };
    match one(root, &op, None).0 {
        Ok(Reply::Stat(info)) => info.fingerprint.unwrap(),
        other => panic!("{other:?}"),
    }
}

fn temps(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".depths-save-")
        })
        .count()
}

#[test]
fn a_put_creates_a_new_file_owned_by_the_helper() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("mods")).unwrap();
    let body: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
    let (answer, rest) = one(dir.path(), &put("mods", "pack.zip", None), Some(&body));
    let Ok(Reply::Written { fingerprint, prior }) = answer else {
        panic!("{answer:?}");
    };
    assert!(!prior);
    rest.done();
    let file = dir.path().join("mods/pack.zip");
    assert_eq!(std::fs::read(&file).unwrap(), body);
    let meta = std::fs::metadata(&file).unwrap();
    assert_eq!(meta.uid(), rustix::process::getuid().as_raw());
    assert_eq!(meta.gid(), rustix::process::getgid().as_raw());
    // Read, not set: setting the umask would race the other tests' file creation.
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let umask = status
        .lines()
        .find_map(|l| l.strip_prefix("Umask:"))
        .map(|v| u32::from_str_radix(v.trim(), 8).unwrap())
        .unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o666 & !umask);
    assert_eq!(fingerprint, self::fingerprint(dir.path(), "mods/pack.zip"));
    assert_eq!(temps(&dir.path().join("mods")), 0);
}

#[test]
fn a_put_over_a_taken_name_needs_a_matching_replace() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("server.properties");
    std::fs::write(&file, b"old").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    let base = fingerprint(dir.path(), "server.properties");
    let stale = Fingerprint::of_content(b"other", 0, 0);

    let mut answers = Script::new()
        .op(&put("", "server.properties", None))
        .data(b"new")
        .op(&put("", "folder", None))
        .data(b"new")
        .op(&put("", "server.properties", Some(stale)))
        .data(b"new")
        .run(dir.path());
    assert_eq!(answers.answer(), Err(HelperError::Exists));
    assert_eq!(answers.answer(), Err(HelperError::Exists));
    assert_eq!(
        answers.answer(),
        Err(HelperError::ChangedOnDisk { current: base })
    );
    answers.done();
    assert_eq!(std::fs::read(&file).unwrap(), b"old");

    let (answer, mut rest) = one(
        dir.path(),
        &put("", "server.properties", Some(base)),
        Some(b"new"),
    );
    assert!(matches!(answer, Ok(Reply::Written { prior: true, .. })));
    assert_eq!(rest.data().unwrap(), b"old");
    rest.done();
    assert_eq!(std::fs::read(&file).unwrap(), b"new");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(temps(dir.path()), 0);
}

#[test]
fn a_put_with_an_invalid_name_is_a_protocol_error_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["../escape", "a/b", "..", ""] {
        let op = format!(
            r#"{{"op":"put","dir":"","name":{}}}"#,
            serde_json::json!(name)
        );
        let mut frame = Vec::new();
        write_frame(&mut frame, FrameKind::Op, op.as_bytes()).unwrap();
        let answers = Script::new().raw(&frame).data(b"x").run(dir.path());
        assert_eq!(answers.code, EXIT_PROTOCOL, "{name:?}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert!(Name::new("../escape").is_err());
}

#[test]
fn a_put_into_a_link_out_or_a_missing_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let tree = build_tree(dir.path());
    let before = snapshot(&tree.sentinels);
    let mut answers = Script::new()
        .op(&put("link_out_dir", "dropped", None))
        .data(b"SENTINEL? no")
        .op(&put("missing", "dropped", None))
        .data(b"x")
        .run(&tree.root);
    assert_eq!(answers.answer(), Err(HelperError::OutsideMount));
    assert_eq!(answers.answer(), Err(HelperError::NotFound));
    answers.done();
    assert_eq!(snapshot(&tree.sentinels), before);
    assert!(!dir.path().join("outside/dropped").exists());
}
