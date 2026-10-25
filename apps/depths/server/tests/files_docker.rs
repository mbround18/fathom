//! Spec 053 T073: the real helper image on a scratch volume and a bind folder (quickstart §4).
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 DEPTHS_HELPER_IMAGE=depths:dev cargo test -p depths-server --test
//! files_docker`. Without `DEPTHS_IT_DOCKER=1` every test returns at once. It creates a volume
//! `depths-it-<random>` and a temp folder, and removes both, and any helper it started, by name.

mod support;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use depths_core::StackName;
use depths_core::ports::VolumeHelper;
use depths_core::role::{Access, Role};
use depths_protocol::{Event, FileOp, FileRefusal, MountId, MountKind, Name, RelPath, UploadFile};
use depths_runtime_cli::helper::{HELPER_IDLE, HelperPool, ImageId, SystemClock};
use depths_runtime_cli::{CliRuntime, CliVolumeHelper};
use depths_server::files;
use depths_server::ops::Who;
use http::StatusCode;
use support::{App, OWNER, app};

const UID: u32 = 1000;

fn enabled() -> bool {
    std::env::var("DEPTHS_IT_DOCKER").as_deref() == Ok("1")
}

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs")
}

fn image() -> String {
    std::env::var("DEPTHS_HELPER_IMAGE").unwrap_or_else(|_| "depths:dev".to_owned())
}

/// The image's id, which is all the adapter accepts.
fn image_id() -> ImageId {
    let out = docker(&["image", "inspect", "--format", "{{.Id}}", &image()]);
    assert!(out.status.success(), "no image {}: build it first", image());
    ImageId::parse(String::from_utf8(out.stdout).unwrap().trim()).unwrap()
}

fn stack() -> StackName {
    StackName::parse("valheim").unwrap()
}

fn admin() -> Who {
    Who {
        session: "test-session".into(),
        user: format!("discord:{OWNER}"),
        access: Access {
            role: Role::Admin,
            stacks: Default::default(),
        },
        actor: serde_json::from_value(serde_json::json!({
            "provider": "discord", "subject": OWNER, "username": OWNER,
        }))
        .unwrap(),
        ip: support::LOOPBACK,
    }
}

fn rel(p: &str) -> RelPath {
    RelPath::parse(p).unwrap()
}

/// The scratch volume, the bind folder and the helpers; removed on drop, by name.
struct Scratch {
    volume: String,
    dir: tempfile::TempDir,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let ids = docker(&[
            "ps",
            "-aq",
            "--filter",
            "label=depths.helper=1",
            "--filter",
            &format!("volume={}", self.volume),
        ]);
        for id in String::from_utf8_lossy(&ids.stdout).split_whitespace() {
            docker(&["rm", "-f", id]);
        }
        let _ = CliVolumeHelper::new(CliRuntime::new("docker"), image_id()).sweep();
        docker(&["volume", "rm", "-f", &self.volume]);
    }
}

/// Seeds `path` in the volume, owned 1000:1000 at 0640, as root in a throwaway container.
fn seed_volume(volume: &str) {
    let out = docker(&[
        "run",
        "--rm",
        "--network",
        "none",
        "--user",
        "0",
        "--entrypoint",
        "/bin/sh",
        "-v",
        &format!("{volume}:/v"),
        &image(),
        "-c",
        "mkdir -p /v/worlds && printf 'name=Valheim\\n' > /v/server.cfg \
         && printf 'seed\\n' > /v/worlds/a.fwl \
         && chown -R 1000:1000 /v && chmod 0640 /v/server.cfg /v/worlds/a.fwl",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn seed_bind(dir: &Path) {
    std::fs::create_dir(dir.join("sub")).unwrap();
    for (p, text) in [("start.cfg", "port=2456\n"), ("sub/b.txt", "inside\n")] {
        std::fs::write(dir.join(p), text).unwrap();
        std::fs::set_permissions(dir.join(p), std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    std::os::unix::fs::symlink("/etc/passwd", dir.join("abs")).unwrap();
}

/// An app whose running `valheim` service mounts the scratch volume at /config and the bind
/// folder at /server, with the real helper behind the pool.
async fn setup() -> (App, Scratch, MountId, MountId) {
    let volume = format!("depths-it-{:016x}", rand_suffix());
    let out = docker(&["volume", "create", &volume]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let scratch = Scratch {
        volume: volume.clone(),
        dir: tempfile::tempdir().unwrap(),
    };
    seed_volume(&volume);
    seed_bind(scratch.dir.path());
    let a = app();
    a.sign_in(OWNER).await;
    let helper: Arc<dyn VolumeHelper> =
        Arc::new(CliVolumeHelper::new(CliRuntime::new("docker"), image_id()));
    let pool = HelperPool::new(helper.clone(), Arc::new(SystemClock), 4, HELPER_IDLE);
    a.state.files.install(helper, pool);
    a.set_containers(vec![support::container(
        "valheim",
        "valheim",
        "running",
        "Up 1 hour",
    )]);
    let bind = scratch.dir.path().display().to_string();
    a.set_inspect(
        "valheim-valheim-id",
        serde_json::json!({"Mounts": [
            {"Type": "volume", "Name": volume, "Source": "/x", "Destination": "/config", "RW": true},
            {"Type": "bind", "Source": bind, "Destination": "/server", "RW": true},
        ]}),
    );
    let mounts = files::list_mounts(&a.state, &admin(), &stack()).unwrap();
    let id = |kind| mounts.iter().find(|m| m.kind == kind).unwrap().id.clone();
    let (vol, bnd) = (id(MountKind::Volume), id(MountKind::Bind));
    (a, scratch, vol, bnd)
}

fn rand_suffix() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(std::process::id())
}

fn names(a: &App, mount: &MountId, dir: &str) -> Vec<(String, u32, u32, u32)> {
    let pages = files::list_dir(&a.state, &admin(), &stack(), mount, &rel(dir)).unwrap();
    let mut v: Vec<_> = pages
        .into_iter()
        .flat_map(|p| p.entries)
        .map(|e| (e.name, e.uid, e.gid, e.mode))
        .collect();
    v.sort();
    v
}

fn confirm(a: &App, op: FileOp) -> Event {
    let p = files::preview(&a.state, &admin(), &stack(), op).unwrap();
    files::confirm(&a.state, &admin(), p.preview_id).unwrap()
}

fn refusal(e: Box<Event>) -> FileRefusal {
    match *e {
        Event::FileRefused(reason) => reason,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_real_helper_lists_saves_downloads_uploads_and_stays_inside() {
    if !enabled() {
        eprintln!(
            "skipped: set DEPTHS_IT_DOCKER=1 (and build {}) to run",
            image()
        );
        return;
    }
    let (a, scratch, vol, bnd) = setup().await;

    // List: the seeded files, as 1000:1000 0640.
    let listed = names(&a, &vol, "");
    assert!(
        listed.contains(&("server.cfg".into(), UID, UID, 0o640)),
        "{listed:?}"
    );
    assert!(listed.iter().any(|e| e.0 == "worlds"), "{listed:?}");
    let listed = names(&a, &bnd, "");
    assert!(
        listed.contains(&("start.cfg".into(), UID, UID, 0o640)),
        "{listed:?}"
    );

    // Save into the volume: the content changes, the owner and mode do not.
    let f = files::read_file(&a.state, &admin(), &stack(), &vol, &rel("server.cfg")).unwrap();
    assert_eq!(f.text.as_deref(), Some("name=Valheim\n"));
    let done = confirm(
        &a,
        FileOp::Save {
            mount: vol.clone(),
            path: rel("server.cfg"),
            text: "name=Depths\n".into(),
            base: f.fingerprint,
            bom: f.bom,
            eol: f.save_eol,
        },
    );
    assert!(matches!(done, Event::FileOpDone { .. }), "{done:?}");
    let f = files::read_file(&a.state, &admin(), &stack(), &vol, &rel("server.cfg")).unwrap();
    assert_eq!(f.text.as_deref(), Some("name=Depths\n"));
    assert!(names(&a, &vol, "").contains(&("server.cfg".into(), UID, UID, 0o640)));

    // Folder download as tar: whole 512-byte blocks naming both seeded files.
    let cookie = a.sign_in(OWNER).await;
    let q: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("mount", vol.as_str())
        .append_pair("path", "")
        .finish();
    let r = a
        .get(
            &format!("/api/v1/stacks/valheim/files/download?{q}"),
            Some(&cookie),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    let tar = axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        !tar.is_empty() && tar.len() % 512 == 0,
        "{} bytes",
        tar.len()
    );
    let has = |needle: &[u8]| tar.windows(needle.len()).any(|w| w == needle);
    assert!(
        has(b"server.cfg") && has(b"worlds/a.fwl"),
        "the tar lacks a seeded file"
    );
    assert!(has(b"name=Depths\n"), "the tar has the saved content");

    // Upload into the bind folder: the new file is there, owned by the folder's owner.
    let name = Name::new("up.txt").unwrap();
    let p = files::preview(
        &a.state,
        &admin(),
        &stack(),
        FileOp::Upload {
            mount: bnd.clone(),
            dir: rel("sub"),
            files: vec![UploadFile {
                name: name.clone(),
                size: 5,
            }],
        },
    )
    .unwrap();
    files::confirm(&a.state, &admin(), p.preview_id).unwrap();
    let mut body = vec![Ok(b"hello".to_vec())].into_iter();
    files::upload(
        &a.state,
        &admin(),
        &stack(),
        p.preview_id,
        &name,
        5,
        &mut || body.next(),
    )
    .unwrap();
    let up = scratch.dir.path().join("sub/up.txt");
    assert_eq!(std::fs::read(&up).unwrap(), b"hello");
    assert_eq!(std::fs::metadata(&up).unwrap().uid(), UID);

    // `../` never parses, so it never reaches the helper.
    assert!(RelPath::parse("../etc/passwd").is_err());
    // An absolute link is outside the mount.
    let e = files::read_file(&a.state, &admin(), &stack(), &bnd, &rel("abs")).unwrap_err();
    assert_eq!(refusal(e), FileRefusal::OutsideMount);
    // A folder swapped for a link to / after it was listed is refused, not followed.
    assert!(names(&a, &bnd, "sub").iter().any(|e| e.0 == "b.txt"));
    let sub = scratch.dir.path().join("sub");
    std::fs::rename(&sub, scratch.dir.path().join("sub.old")).unwrap();
    std::os::unix::fs::symlink("/etc", &sub).unwrap();
    let e = files::read_file(&a.state, &admin(), &stack(), &bnd, &rel("sub/passwd")).unwrap_err();
    assert_eq!(refusal(e), FileRefusal::OutsideMount);
    drop(scratch);
}
