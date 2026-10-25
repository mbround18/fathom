//! T032: `FsStackStore` on temp dirs. Each test builds its own stacks root.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use depths_core::StackName;
use depths_core::discovery::ComposeFile;
use depths_core::hash::content_hash_of;
use depths_core::ports::{DefinitionState, StackStore, StoreError};
use depths_stacks_fs::{DEFAULT_MAX_FILE_BYTES, FsStackStore};
use tempfile::TempDir;

const COMPOSE: &str = "services:\n  app:\n    image: busybox\n";

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("stacks")).unwrap();
        fs::create_dir(dir.path().join("outside")).unwrap();
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("stacks")
    }

    fn outside(&self) -> PathBuf {
        self.dir.path().join("outside")
    }

    fn stack(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let d = self.root().join(name);
        fs::create_dir_all(&d).unwrap();
        for (f, body) in files {
            fs::write(d.join(f), body).unwrap();
        }
        d
    }

    fn store(&self) -> FsStackStore {
        FsStackStore::open(self.root()).unwrap()
    }
}

fn name(s: &str) -> StackName {
    StackName::parse(s).unwrap()
}

/// File permissions do not stop root, so the tests that rely on them skip there.
fn is_root() -> bool {
    fs::metadata("/proc/self")
        .map(|m| m.uid() == 0)
        .unwrap_or(false)
}

/// Every path under `dir` with its bytes, mode and mtime.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32, SystemTime)> {
    let mut out = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let m = fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                todo.push(p.clone());
            }
            let body = if m.is_file() {
                fs::read(&p).unwrap()
            } else {
                vec![]
            };
            out.insert(p, (body, m.mode(), m.modified().unwrap()));
        }
    }
    out
}

// -- FR-005: nothing outside root/<name> ----------------------------------------------------

#[test]
fn names_that_would_leave_the_root_never_reach_the_store() {
    for bad in ["../x", "..", "/etc", "a/b", "web/../../x", ""] {
        assert!(StackName::parse(bad).is_err(), "{bad:?} parsed");
    }
    let f = Fixture::new();
    assert_eq!(f.store().read(&name("missing")), Err(StoreError::NotFound));
}

#[test]
fn a_compose_file_symlinked_out_of_the_stack_is_refused() {
    let f = Fixture::new();
    fs::write(f.outside().join("secret.yaml"), COMPOSE).unwrap();
    let d = f.stack("web", &[]);
    symlink("../../outside/secret.yaml", d.join("compose.yaml")).unwrap();
    let abs = f.stack("abs", &[]);
    symlink(f.outside().join("secret.yaml"), abs.join("compose.yaml")).unwrap();

    let store = f.store();
    for n in ["web", "abs"] {
        assert!(
            matches!(store.read(&name(n)), Err(StoreError::Escapes { .. })),
            "{n}"
        );
        let e = store.entry(&name(n)).unwrap();
        assert!(e.read_error.is_some(), "{n}: {e:?}");
        assert!(!e.editable());
    }
    let listed: Vec<String> = store
        .list()
        .unwrap()
        .stacks
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(listed, vec!["abs", "web"]);
}

#[test]
fn an_env_file_symlinked_out_of_the_stack_is_refused() {
    let f = Fixture::new();
    fs::write(f.outside().join("creds"), "TOKEN=x\n").unwrap();
    let d = f.stack("web", &[("compose.yaml", COMPOSE.as_bytes())]);
    symlink("../../outside/creds", d.join(".env")).unwrap();
    assert!(matches!(
        f.store().read(&name("web")),
        Err(StoreError::Escapes { .. })
    ));
}

#[test]
fn a_symlink_to_a_file_in_the_same_stack_is_followed_and_kept() {
    let f = Fixture::new();
    let d = f.stack("web", &[("real.yaml", COMPOSE.as_bytes())]);
    symlink("real.yaml", d.join("compose.yaml")).unwrap();

    let store = f.store();
    let files = store.read(&name("web")).unwrap();
    assert_eq!(files.compose, COMPOSE);

    let next = "services: {}\n";
    store
        .write_atomic(&name("web"), next, None, &files.content_hash)
        .unwrap();
    assert!(
        fs::symlink_metadata(d.join("compose.yaml"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(d.join("real.yaml")).unwrap(), next);
}

#[test]
fn a_symlinked_stack_directory_is_listed_read_only_and_never_followed() {
    let f = Fixture::new();
    let target = f.outside().join("elsewhere");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("compose.yaml"), COMPOSE).unwrap();
    symlink(&target, f.root().join("linked")).unwrap();

    let store = f.store();
    let listing = store.list().unwrap();
    let e = listing.stacks.iter().find(|e| e.name == "linked").unwrap();
    assert!(e.read_only_reason.as_deref().unwrap().contains("symlink"));
    assert!(!e.editable());

    let n = name("linked");
    assert!(matches!(store.read(&n), Err(StoreError::Escapes { .. })));
    let h = content_hash_of(ComposeFile::ComposeYaml, COMPOSE.as_bytes(), None);
    assert!(matches!(
        store.write_atomic(&n, "x", None, &h),
        Err(StoreError::Escapes { .. })
    ));
    assert!(matches!(
        store.delete_dir(&n),
        Err(StoreError::Escapes { .. })
    ));
    assert_eq!(
        fs::read_to_string(target.join("compose.yaml")).unwrap(),
        COMPOSE
    );
}

// -- Writing ------------------------------------------------------------------------------

#[test]
fn write_atomic_writes_the_exact_bytes_and_keeps_mode_and_owner() {
    let f = Fixture::new();
    let d = f.stack(
        "web",
        &[("compose.yaml", COMPOSE.as_bytes()), (".env", b"A=1\n")],
    );
    fs::set_permissions(d.join("compose.yaml"), fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(d.join(".env"), fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::metadata(d.join("compose.yaml")).unwrap();

    let store = f.store();
    let base = store.read(&name("web")).unwrap().content_hash;
    let compose = "\u{feff}services:\r\n  app: # keep me   \r\n    image: busybox\t\r\n\r\n";
    let env = "A=2\r\n# note  \n";
    let new = store
        .write_atomic(&name("web"), compose, Some(env), &base)
        .unwrap();

    assert_eq!(
        fs::read(d.join("compose.yaml")).unwrap(),
        compose.as_bytes()
    );
    assert_eq!(fs::read(d.join(".env")).unwrap(), env.as_bytes());
    let after = fs::metadata(d.join("compose.yaml")).unwrap();
    assert_eq!(after.mode() & 0o7777, 0o640);
    assert_eq!(fs::metadata(d.join(".env")).unwrap().mode() & 0o7777, 0o600);
    assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
    assert_ne!(
        after.ino(),
        before.ino(),
        "replaced by rename, not rewritten in place"
    );

    let leftovers: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n != "compose.yaml" && n != ".env")
        .collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");

    assert_eq!(
        new,
        content_hash_of(
            ComposeFile::ComposeYaml,
            compose.as_bytes(),
            Some(env.as_bytes())
        )
    );
    assert_eq!(store.entry(&name("web")).unwrap().content_hash, Some(new));
}

#[test]
fn write_atomic_keeps_the_compose_files_name() {
    let f = Fixture::new();
    let d = f.stack("web", &[("docker-compose.yml", COMPOSE.as_bytes())]);
    let store = f.store();
    let files = store.read(&name("web")).unwrap();
    assert_eq!(files.compose_file, ComposeFile::DockerComposeYml);
    store
        .write_atomic(&name("web"), "services: {}\n", None, &files.content_hash)
        .unwrap();
    assert_eq!(
        fs::read_to_string(d.join("docker-compose.yml")).unwrap(),
        "services: {}\n"
    );
    assert!(!d.join("compose.yaml").exists());
}

#[test]
fn a_stale_base_hash_is_a_conflict_carrying_the_current_files() {
    let f = Fixture::new();
    let d = f.stack("web", &[("compose.yaml", COMPOSE.as_bytes())]);
    let store = f.store();
    let stale = store.read(&name("web")).unwrap().content_hash;
    fs::write(d.join("compose.yaml"), "services: {b: {}}\n").unwrap();

    match store.write_atomic(&name("web"), "mine\n", None, &stale) {
        Err(StoreError::Conflict { current }) => {
            assert_eq!(current.compose, "services: {b: {}}\n");
            assert_ne!(current.content_hash, stale);
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(d.join("compose.yaml")).unwrap(),
        "services: {b: {}}\n"
    );
}

#[test]
fn an_empty_env_with_no_env_file_creates_nothing() {
    let f = Fixture::new();
    let d = f.stack("web", &[("compose.yaml", COMPOSE.as_bytes())]);
    let store = f.store();
    let base = store.read(&name("web")).unwrap().content_hash;
    let h = store
        .write_atomic(&name("web"), COMPOSE, Some(""), &base)
        .unwrap();
    assert!(!d.join(".env").exists());
    assert_eq!(h, base, "the same bytes hash the same");

    store.create(&name("fresh"), COMPOSE, Some("")).unwrap();
    assert!(!f.root().join("fresh/.env").exists());
}

#[test]
fn an_empty_env_over_an_existing_file_empties_it() {
    let f = Fixture::new();
    let d = f.stack(
        "web",
        &[("compose.yaml", COMPOSE.as_bytes()), (".env", b"A=1\n")],
    );
    let store = f.store();
    let base = store.read(&name("web")).unwrap().content_hash;
    store
        .write_atomic(&name("web"), COMPOSE, Some(""), &base)
        .unwrap();
    assert_eq!(fs::read(d.join(".env")).unwrap(), b"");
}

#[test]
fn create_makes_the_directory_and_refuses_an_existing_one() {
    let f = Fixture::new();
    f.stack("taken", &[]);
    let store = f.store();

    let h = store
        .create(&name("fresh"), COMPOSE, Some("A=1\n"))
        .unwrap();
    let d = f.root().join("fresh");
    assert_eq!(fs::read_to_string(d.join("compose.yaml")).unwrap(), COMPOSE);
    assert_eq!(
        fs::metadata(d.join("compose.yaml")).unwrap().mode() & 0o7777,
        0o644
    );
    assert_eq!(fs::metadata(d.join(".env")).unwrap().mode() & 0o7777, 0o600);
    assert_eq!(h, store.read(&name("fresh")).unwrap().content_hash);

    assert_eq!(
        store.create(&name("taken"), COMPOSE, None),
        Err(StoreError::AlreadyExists)
    );
    assert_eq!(
        store.create(&name("fresh"), COMPOSE, None),
        Err(StoreError::AlreadyExists)
    );
}

#[test]
fn create_refuses_a_name_taken_by_a_symlink() {
    let f = Fixture::new();
    symlink(f.outside(), f.root().join("web")).unwrap();
    assert_eq!(
        f.store().create(&name("web"), COMPOSE, None),
        Err(StoreError::AlreadyExists)
    );
    assert!(!f.outside().join("compose.yaml").exists());
}

#[test]
fn delete_dir_removes_the_stack_and_never_follows_a_symlink_inside_it() {
    let f = Fixture::new();
    fs::write(f.outside().join("keep"), "x").unwrap();
    let d = f.stack("web", &[("compose.yaml", COMPOSE.as_bytes())]);
    fs::create_dir(d.join("data")).unwrap();
    symlink(f.outside(), d.join("data/out")).unwrap();

    let store = f.store();
    store.delete_dir(&name("web")).unwrap();
    assert!(!d.exists());
    assert!(f.outside().join("keep").exists());
    assert_eq!(store.delete_dir(&name("web")), Err(StoreError::NotFound));
}

// -- Size, hash, mtime --------------------------------------------------------------------

#[test]
fn a_file_over_the_limit_is_not_loaded() {
    assert_eq!(DEFAULT_MAX_FILE_BYTES, 1024 * 1024);
    let f = Fixture::new();
    f.stack("big", &[("compose.yaml", &[b'#'; 40])]);
    f.stack(
        "bigenv",
        &[("compose.yaml", b"services: {}\n"), (".env", &[b'#'; 40])],
    );
    let store = f.store().with_max_file_bytes(32);

    assert_eq!(
        store.read(&name("big")),
        Err(StoreError::TooLarge {
            file: "compose.yaml".into(),
            bytes: 40,
            limit: 32
        })
    );
    assert!(matches!(
        store.read(&name("bigenv")),
        Err(StoreError::TooLarge {
            bytes: 40,
            limit: 32,
            ..
        })
    ));
    for n in ["big", "bigenv"] {
        let e = store.entry(&name(n)).unwrap();
        assert!(e.too_large && !e.editable(), "{n}: {e:?}");
        assert_eq!(e.definition, DefinitionState::NotLoaded);
    }
}

#[test]
fn the_content_hash_changes_when_either_file_changes() {
    let f = Fixture::new();
    let d = f.stack("web", &[("compose.yaml", COMPOSE.as_bytes())]);
    let store = f.store();
    let hash = || store.entry(&name("web")).unwrap().content_hash.unwrap();

    let a = hash();
    assert_eq!(a, hash());
    fs::write(d.join(".env"), "").unwrap();
    let b = hash();
    fs::write(d.join(".env"), "A=1").unwrap();
    let c = hash();
    fs::write(d.join("compose.yaml"), "services: {}\n").unwrap();
    let e = hash();
    assert!(a != b && b != c && c != e && a != e);
}

#[test]
fn modified_at_is_the_latest_mtime() {
    let f = Fixture::new();
    let d = f.stack(
        "web",
        &[("compose.yaml", COMPOSE.as_bytes()), (".env", b"A=1")],
    );
    let t1 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let t2 = t1 + Duration::from_secs(3600);
    let set = |p: PathBuf, t| {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap()
    };
    set(d.join("compose.yaml"), t1);
    set(d.join(".env"), t2);
    assert_eq!(f.store().entry(&name("web")).unwrap().modified_at, Some(t2));
    set(d.join("compose.yaml"), t2 + Duration::from_secs(1));
    assert_eq!(
        f.store().entry(&name("web")).unwrap().modified_at,
        Some(t2 + Duration::from_secs(1))
    );
}

// -- Listing ------------------------------------------------------------------------------

#[test]
fn list_shows_every_directory_with_a_compose_file() {
    let f = Fixture::new();
    f.stack(
        "web",
        &[
            ("compose.yaml", COMPOSE.as_bytes()),
            ("docker-compose.yml", b"x"),
        ],
    );
    f.stack("Bad Name", &[("compose.yml", COMPOSE.as_bytes())]);
    f.stack("empty", &[("notes.txt", b"hi")]);
    f.stack("broken", &[("compose.yaml", b"services: [\n")]);
    fs::write(f.root().join("stray.yaml"), COMPOSE).unwrap();

    let store = f.store();
    let listing = store.list().unwrap();
    assert!(!listing.global_env);
    let names: Vec<&str> = listing.stacks.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Bad Name", "broken", "web"]);

    let bad = &listing.stacks[0];
    assert!(!bad.valid_name && !bad.actionable());
    assert!(matches!(
        listing.stacks[1].definition,
        DefinitionState::Unparseable(_)
    ));
    let web = &listing.stacks[2];
    assert_eq!(web.compose_file, Some(ComposeFile::ComposeYaml));
    assert_eq!(web.shadowed_files, vec![ComposeFile::DockerComposeYml]);
    assert!(!web.has_env);
    match &web.definition {
        DefinitionState::Parsed(s) => assert_eq!(s[0].name, "app"),
        other => panic!("{other:?}"),
    }
    assert!(web.editable());

    assert_eq!(store.entry(&name("empty")), Err(StoreError::NotAStack));

    fs::write(f.root().join("global.env"), "TZ=UTC\n").unwrap();
    assert!(store.list().unwrap().global_env);
    assert!(store.global_env());
}

#[test]
fn a_compose_file_that_is_not_utf8_is_not_text() {
    let f = Fixture::new();
    f.stack("web", &[("compose.yaml", b"services: {}\n\xff\xfe")]);
    assert_eq!(
        f.store().read(&name("web")),
        Err(StoreError::NotText {
            file: "compose.yaml".into()
        })
    );
}

#[test]
fn reading_writes_nothing() {
    let f = Fixture::new();
    f.stack(
        "web",
        &[("compose.yaml", COMPOSE.as_bytes()), (".env", b"A=1")],
    );
    f.stack("big", &[("compose.yaml", &[b'#'; 64])]);
    let before = snapshot(f.dir.path());
    let store = f.store().with_max_file_bytes(48);
    store.list().unwrap();
    store.entry(&name("web")).unwrap();
    store.read(&name("web")).unwrap();
    let _ = store.read(&name("big"));
    let _ = store.read(&name("missing"));
    store.global_env();
    assert_eq!(before, snapshot(f.dir.path()));
}

// -- Permissions --------------------------------------------------------------------------

#[test]
fn what_depths_cannot_read_is_listed_with_a_reason_and_no_edit() {
    if is_root() {
        return;
    }
    let f = Fixture::new();
    let locked = f.stack("locked", &[("compose.yaml", COMPOSE.as_bytes())]);
    let secret = f.stack("secret", &[("compose.yaml", COMPOSE.as_bytes())]);
    fs::set_permissions(
        secret.join("compose.yaml"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let listing = f.store().list();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let listing = listing.unwrap();
    for n in ["locked", "secret"] {
        let e = listing.stacks.iter().find(|e| e.name == n).unwrap();
        assert!(e.read_error.is_some(), "{n}: {e:?}");
        assert!(!e.editable() && !e.actionable());
    }
    assert!(f.store().read(&name("secret")).is_err());
}

#[test]
fn what_depths_cannot_write_is_listed_read_only_and_refused() {
    if is_root() {
        return;
    }
    let f = Fixture::new();
    let ro_dir = f.stack("rodir", &[("compose.yaml", COMPOSE.as_bytes())]);
    let ro_file = f.stack("rofile", &[("compose.yaml", COMPOSE.as_bytes())]);
    fs::set_permissions(
        ro_file.join("compose.yaml"),
        fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    fs::set_permissions(&ro_dir, fs::Permissions::from_mode(0o555)).unwrap();

    let store = f.store();
    let results: Vec<_> = ["rodir", "rofile"]
        .into_iter()
        .map(|n| {
            let e = store.entry(&name(n)).unwrap();
            let base = e.content_hash.clone().unwrap();
            (e, store.write_atomic(&name(n), "x\n", None, &base))
        })
        .collect();
    fs::set_permissions(&ro_dir, fs::Permissions::from_mode(0o755)).unwrap();

    for (e, w) in results {
        assert!(e.read_only_reason.is_some(), "{e:?}");
        assert!(e.actionable() && !e.editable());
        assert!(matches!(w, Err(StoreError::ReadOnly { .. })), "{w:?}");
    }
    assert_eq!(
        fs::read_to_string(ro_dir.join("compose.yaml")).unwrap(),
        COMPOSE
    );
    assert_eq!(
        fs::read_to_string(ro_file.join("compose.yaml")).unwrap(),
        COMPOSE
    );
}
