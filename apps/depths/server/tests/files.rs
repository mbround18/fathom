//! Spec 053: the file gate (T022), the helper's refusals (T022) and mount discovery (T024).

mod support;

use std::collections::BTreeSet;
use std::net::IpAddr;

use depths_core::StackName;
use depths_core::role::{Access, CommandKind, Role};
use depths_protocol::{
    Actor, AuditAction, AuditOutcome, Browsable, DenyReason, Event, FileRefusal, Fingerprint,
    Mount, MountKind, MountService, MountState, NotBrowsableCode, RelPath, WriteBlocked,
};
use depths_server::config::FilesMode;
use depths_server::files::{self, At};
use depths_server::ops::Who;
use depths_volumes::op::HelperError;
use support::{App, OWNER, app, app_with_env};

const AWAY: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 7));
const CAFE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

fn stack() -> StackName {
    StackName::parse("valheim").unwrap()
}

fn actor(subject: &str) -> Actor {
    serde_json::from_value(serde_json::json!({
        "provider": "discord", "subject": subject, "username": subject,
    }))
    .unwrap()
}

fn who(subject: &str, role: Role, stacks: &[&str], ip: IpAddr) -> Who {
    Who {
        session: "test-session".into(),
        user: format!("discord:{subject}"),
        access: Access {
            role,
            stacks: stacks
                .iter()
                .map(|s| StackName::parse(s).unwrap())
                .collect::<BTreeSet<_>>(),
        },
        actor: actor(subject),
        ip,
    }
}

fn admin() -> Who {
    who(OWNER, Role::Admin, &[], support::LOOPBACK)
}

fn service(name: &str, read_only: bool) -> MountService {
    MountService {
        service: name.into(),
        destination: "/data".into(),
        read_only,
    }
}

fn mount(state: MountState, browsable: Browsable, read_only: bool) -> Mount {
    Mount::new(
        MountKind::Volume,
        "valheim_saves".into(),
        vec![service("valheim", read_only)],
        state,
        browsable,
        vec![],
    )
}

fn ready() -> Mount {
    mount(MountState::Ready, Browsable::yes(), false)
}

fn path() -> RelPath {
    RelPath::parse("worlds/a.db").unwrap()
}

fn refused(a: &App) -> Vec<depths_protocol::AuditEntry> {
    a.audit_entries()
        .into_iter()
        .filter(|e| e.action == AuditAction::FilesRefused)
        .rev()
        .collect()
}

fn reason_of(e: &depths_protocol::AuditEntry) -> String {
    match &e.outcome {
        AuditOutcome::Denied { reason } => reason.clone(),
        other => panic!("not a refusal: {other:?}"),
    }
}

#[allow(clippy::result_large_err)]
fn gate(a: &App, w: &Who, kind: CommandKind, at: Option<(&Mount, &RelPath)>) -> Result<(), Event> {
    files::gate(
        &a.state,
        w,
        &stack(),
        kind,
        "ReadFile",
        at.map(|(mount, path)| At { mount, path }),
    )
    .map_err(|e| *e)
}

#[tokio::test]
async fn an_admin_on_a_ready_mount_passes_every_kind() {
    let a = app();
    a.sign_in(OWNER).await;
    let (m, p) = (ready(), path());
    for kind in [
        CommandKind::FilesList,
        CommandKind::FilesRead,
        CommandKind::FilesWrite,
    ] {
        assert_eq!(gate(&a, &admin(), kind, Some((&m, &p))), Ok(()), "{kind:?}");
    }
    assert!(refused(&a).is_empty());
}

#[test]
fn off_is_feature_disabled_before_anything_else() {
    let a = app_with_env(&[("DEPTHS_FEATURE_FILES", "off")], |_| {});
    // Even a viewer with no access hears `feature_disabled`, not `not_permitted`.
    let w = who("3003", Role::Pending, &[], support::LOOPBACK);
    let r = gate(&a, &w, CommandKind::FilesList, None);
    assert_eq!(
        r,
        Err(Event::Denied {
            reason: DenyReason::FeatureDisabled,
            detail: None
        })
    );
    let entries = refused(&a);
    assert_eq!(entries.len(), 1);
    assert_eq!(reason_of(&entries[0]), "ReadFile: feature_disabled");
    assert_eq!(entries[0].stack.as_deref(), Some("valheim"));
}

#[test]
fn forced_off_is_feature_disabled_and_keeps_its_reason() {
    let a = app();
    a.state.files.force_off("no helper image");
    assert_eq!(a.state.files.mode(), FilesMode::Off);
    assert_eq!(a.state.files.reason().as_deref(), Some("no helper image"));
    assert!(matches!(
        gate(&a, &admin(), CommandKind::FilesRead, None),
        Err(Event::Denied {
            reason: DenyReason::FeatureDisabled,
            ..
        })
    ));
}

#[tokio::test]
async fn read_only_mode_lets_reads_through_and_refuses_writes() {
    let a = app_with_env(&[("DEPTHS_FEATURE_FILES", "read_only")], |_| {});
    a.sign_in(OWNER).await;
    let (m, p) = (ready(), path());
    assert_eq!(
        gate(&a, &admin(), CommandKind::FilesList, Some((&m, &p))),
        Ok(())
    );
    assert_eq!(
        gate(&a, &admin(), CommandKind::FilesRead, Some((&m, &p))),
        Ok(())
    );
    assert_eq!(
        gate(&a, &admin(), CommandKind::FilesWrite, Some((&m, &p))),
        Err(Event::FileRefused(FileRefusal::FeatureReadOnly))
    );
    let entries = refused(&a);
    assert_eq!(entries.len(), 1);
    assert_eq!(reason_of(&entries[0]), "feature_read_only");
    let place = entries[0].file.as_ref().unwrap();
    assert_eq!(place.op.as_deref(), Some("ReadFile"));
    assert_eq!(place.mount, m.id);
    assert_eq!(place.path, p);
    assert_eq!(place.service.as_deref(), Some("valheim"));
}

#[test]
fn a_viewer_or_an_unassigned_user_is_not_permitted() {
    let a = app();
    let (m, p) = (ready(), path());
    for w in [
        who("3003", Role::Pending, &[], support::LOOPBACK),
        who("3004", Role::User, &["other"], support::LOOPBACK),
    ] {
        assert_eq!(
            gate(&a, &w, CommandKind::FilesList, Some((&m, &p))),
            Err(Event::Denied {
                reason: DenyReason::NotPermitted,
                detail: None
            })
        );
    }
    let entries = refused(&a);
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| reason_of(e) == "not_permitted"));
}

#[tokio::test]
async fn an_assigned_user_passes() {
    let a = app();
    a.sign_in("3004").await;
    let (m, p) = (ready(), path());
    let w = who("3004", Role::User, &["valheim"], support::LOOPBACK);
    assert_eq!(
        gate(&a, &w, CommandKind::FilesWrite, Some((&m, &p))),
        Ok(())
    );
}

#[tokio::test]
async fn an_untrusted_network_may_list_but_not_read_or_write() {
    let a = app();
    // Signing in from AWAY makes it the owner's first (trusted) network; CAFE is unknown.
    a.sign_in_from(AWAY, OWNER).await;
    let w = who(OWNER, Role::Admin, &[], CAFE);
    let (m, p) = (ready(), path());
    assert_eq!(gate(&a, &w, CommandKind::FilesList, Some((&m, &p))), Ok(()));
    for kind in [CommandKind::FilesRead, CommandKind::FilesWrite] {
        assert_eq!(
            gate(&a, &w, kind, Some((&m, &p))),
            Err(Event::Denied {
                reason: DenyReason::UntrustedNetwork,
                detail: None
            }),
            "{kind:?}"
        );
    }
    // The network gate audits its own refusals; the file gate adds none.
    assert!(refused(&a).is_empty());
    let network = a
        .audit_entries()
        .into_iter()
        .filter(|e| e.action == AuditAction::NetworkRefused)
        .count();
    assert_eq!(network, 2);
}

#[tokio::test]
async fn mount_state_browsability_and_read_only_refuse_in_order() {
    let a = app();
    a.sign_in(OWNER).await;
    let p = path();
    let risky = Browsable::no(
        NotBrowsableCode::RiskyBind,
        "mounts the host's whole filesystem",
    );
    let cases: Vec<(Mount, CommandKind, FileRefusal)> = vec![
        (
            mount(MountState::NotCreated, Browsable::yes(), false),
            CommandKind::FilesList,
            FileRefusal::NotCreated,
        ),
        (
            mount(MountState::Missing, Browsable::yes(), false),
            CommandKind::FilesRead,
            FileRefusal::Missing,
        ),
        (
            // A state refusal wins over browsability.
            mount(MountState::Missing, risky.clone(), false),
            CommandKind::FilesList,
            FileRefusal::Missing,
        ),
        (
            mount(MountState::Ready, risky.clone(), false),
            CommandKind::FilesList,
            FileRefusal::NotBrowsable {
                reason: risky.reason.clone().unwrap(),
            },
        ),
        (
            mount(MountState::Ready, Browsable::yes(), true),
            CommandKind::FilesWrite,
            FileRefusal::ReadOnlyMount,
        ),
    ];
    for (m, kind, want) in &cases {
        assert_eq!(
            gate(&a, &admin(), *kind, Some((m, &p))),
            Err(Event::FileRefused(want.clone())),
            "{want:?}"
        );
    }
    let unresolved = mount(MountState::Unresolved, Browsable::yes(), false);
    assert!(matches!(
        gate(
            &a,
            &admin(),
            CommandKind::FilesList,
            Some((&unresolved, &p))
        ),
        Err(Event::FileRefused(FileRefusal::NotBrowsable { .. }))
    ));
    // A read-only mount still reads.
    let ro = mount(MountState::Ready, Browsable::yes(), true);
    assert_eq!(
        gate(&a, &admin(), CommandKind::FilesRead, Some((&ro, &p))),
        Ok(())
    );

    let reasons: Vec<String> = refused(&a).iter().map(reason_of).collect();
    assert_eq!(
        reasons,
        [
            "not_created",
            "missing",
            "missing",
            "not_browsable",
            "read_only_mount",
            "not_browsable"
        ]
    );
}

#[tokio::test]
async fn every_helper_error_maps_to_its_refusal_and_one_audit_entry() {
    let a = app();
    a.sign_in(OWNER).await;
    let (m, p) = (ready(), path());
    let current = Fingerprint::new([0xaa; 32], 5, 1, 2);
    let cases = vec![
        (HelperError::OutsideMount, "outside_mount"),
        (
            HelperError::WriteBlocked {
                reason: WriteBlocked::HardLink,
            },
            "write_blocked",
        ),
        (HelperError::ChangedOnDisk { current }, "changed_on_disk"),
        (HelperError::OwnerChanged, "owner_changed"),
        (HelperError::TooLarge, "too_large"),
        (HelperError::NotText, "not_text"),
        (HelperError::Exists, "exists"),
        (HelperError::NotFound, "not_found"),
        (HelperError::InvalidName, "invalid_name"),
        (HelperError::MixedOwners, "mixed_owners"),
        (
            HelperError::Partial {
                removed: 3,
                failed_at: "b/c".into(),
                errno: "EACCES".into(),
                message: "Permission denied".into(),
            },
            "partial",
        ),
        (
            HelperError::OsError {
                errno: "ENOSPC".into(),
                message: "No space left on device".into(),
            },
            "os_error",
        ),
        (HelperError::Unsupported, "helper_unavailable"),
        (
            HelperError::Protocol {
                message: "bad frame".into(),
            },
            "helper_unavailable",
        ),
    ];
    for (err, code) in &cases {
        let refusal = files::refusal_from_helper(err.clone());
        assert_eq!(files::refusal_code(&refusal), *code, "{err:?}");
        let ev = files::refuse(
            &a.state,
            &admin(),
            &stack(),
            "ReadFile",
            Some(At {
                mount: &m,
                path: &p,
            }),
            refusal.clone(),
        );
        assert_eq!(ev, Event::FileRefused(refusal));
    }
    assert_eq!(
        files::refusal_from_helper(HelperError::ChangedOnDisk { current }),
        FileRefusal::ChangedOnDisk { current }
    );
    let entries = refused(&a);
    assert_eq!(entries.len(), cases.len());
    for (e, (_, code)) in entries.iter().zip(&cases) {
        assert_eq!(reason_of(e), *code);
        let place = e.file.as_ref().unwrap();
        assert_eq!(place.path, p);
        assert_eq!(place.op.as_deref(), Some("ReadFile"));
    }
}

// ---------------------------------------------------------------------------
// Mount discovery (T024)
// ---------------------------------------------------------------------------

use depths_server::files::{Found, Seen, discover};
use depths_volumes::mount::HelperSource;
use std::path::{Path, PathBuf};

fn inspected(service: &str, running: bool, mounts: serde_json::Value) -> Seen {
    Seen {
        service: service.into(),
        running,
        mounts: serde_json::from_value(mounts).unwrap(),
    }
}

fn found_all(_: &HelperSource) -> Found {
    Found::Dir
}

const DIR: &str = "/stacks/valheim";

#[test]
fn an_inspected_volume_is_listed_by_name_and_ready() {
    let seen = [inspected(
        "valheim",
        true,
        serde_json::json!([{
            "Type": "volume", "Name": "valheim_saves",
            "Source": "/var/lib/docker/volumes/valheim_saves/_data",
            "Destination": "/config", "RW": true,
        }]),
    )];
    let d = discover("valheim", Path::new(DIR), &seen, None, &found_all);
    assert_eq!(d.mounts.len(), 1);
    let m = &d.mounts[0];
    assert_eq!(
        (m.kind, m.source.as_str()),
        (MountKind::Volume, "valheim_saves")
    );
    assert_eq!(m.state, MountState::Ready);
    assert!(m.browsable.ok && m.writable);
    assert_eq!(m.running, vec!["valheim".to_owned()]);
    assert_eq!(
        d.sources.get(&m.id),
        Some(&HelperSource::Volume("valheim_saves".into()))
    );
}

#[test]
fn a_relative_bind_from_compose_resolves_against_the_stack_dir() {
    let compose = serde_json::json!({
        "name": "valheim",
        "services": {"valheim": {"volumes": [
            {"type": "bind", "source": "./server", "target": "/opt/valheim", "read_only": true},
        ]}},
    });
    let d = discover("valheim", Path::new(DIR), &[], Some(&compose), &found_all);
    let m = &d.mounts[0];
    assert_eq!(m.source, "/stacks/valheim/server");
    assert_eq!(m.state, MountState::Ready);
    assert!(m.browsable.ok && !m.writable);
    assert!(m.running.is_empty());
    assert_eq!(
        d.sources.get(&m.id),
        Some(&HelperSource::Bind(PathBuf::from("/stacks/valheim/server")))
    );
}

#[test]
fn a_bind_of_the_host_root_is_listed_but_not_browsable() {
    let seen = [inspected(
        "agent",
        true,
        serde_json::json!([{"Type": "bind", "Source": "/", "Destination": "/host", "RW": true}]),
    )];
    let d = discover("valheim", Path::new(DIR), &seen, None, &found_all);
    let m = &d.mounts[0];
    assert!(!m.browsable.ok);
    let why = m.browsable.reason.as_ref().unwrap();
    assert_eq!(why.code, NotBrowsableCode::RiskyBind);
    assert!(why.message.contains("whole filesystem"), "{}", why.message);
}

#[test]
fn tmpfs_is_listed_by_destination_with_no_source() {
    let seen = [inspected(
        "valheim",
        true,
        serde_json::json!([{"Type": "tmpfs", "Source": "", "Destination": "/tmp", "RW": true}]),
    )];
    let d = discover("valheim", Path::new(DIR), &seen, None, &found_all);
    let m = &d.mounts[0];
    assert_eq!((m.kind, m.source.as_str()), (MountKind::Tmpfs, "/tmp"));
    assert_eq!(
        m.browsable.reason.as_ref().unwrap().code,
        NotBrowsableCode::Tmpfs
    );
    assert!(d.sources.is_empty());
}

#[test]
fn a_mount_shared_by_services_is_merged() {
    let vol = |rw| {
        serde_json::json!([{
            "Type": "volume", "Name": "valheim_saves", "Source": "/x",
            "Destination": "/saves", "RW": rw,
        }])
    };
    let seen = [
        inspected("valheim", true, vol(true)),
        inspected("backup", false, vol(false)),
    ];
    let d = discover("valheim", Path::new(DIR), &seen, None, &found_all);
    assert_eq!(d.mounts.len(), 1);
    let m = &d.mounts[0];
    let names: Vec<_> = m.services.iter().map(|s| s.service.as_str()).collect();
    assert_eq!(names, ["valheim", "backup"]);
    assert!(m.writable);
    assert_eq!(m.running, vec!["valheim".to_owned()]);
}

#[test]
fn a_never_deployed_stack_lists_its_volumes_as_not_created() {
    let compose = serde_json::json!({
        "name": "valheim",
        "services": {"valheim": {"volumes": [
            {"type": "volume", "source": "saves", "target": "/saves"},
            {"type": "volume", "source": "named", "target": "/named"},
            {"type": "volume", "target": "/anon"},
            {"type": "bind", "source": "${DATA}/x", "target": "/data"},
        ]}},
        "volumes": {"saves": {}, "named": {"name": "shared-named"}},
    });
    let missing = |_: &HelperSource| Found::Missing;
    let d = discover("valheim", Path::new(DIR), &[], Some(&compose), &missing);
    let got: Vec<_> = d
        .mounts
        .iter()
        .map(|m| (m.source.as_str(), m.state))
        .collect();
    assert_eq!(
        got,
        [
            ("valheim_saves", MountState::NotCreated),
            ("shared-named", MountState::NotCreated),
            ("${DATA}/x", MountState::Unresolved),
        ]
    );
    assert_eq!(d.sources.len(), 2, "an unresolved bind has no source");
}

#[test]
fn a_source_gone_since_deploy_is_missing_and_a_file_bind_is_not_browsable() {
    let seen = [inspected(
        "valheim",
        false,
        serde_json::json!([
            {"Type": "volume", "Name": "gone", "Source": "/x", "Destination": "/a", "RW": true},
            {"Type": "bind", "Source": "/srv/app.cfg", "Destination": "/app.cfg", "RW": true},
        ]),
    )];
    let probe = |s: &HelperSource| match s {
        HelperSource::Volume(_) => Found::Missing,
        HelperSource::Bind(_) => Found::File,
    };
    let d = discover("valheim", Path::new(DIR), &seen, None, &probe);
    assert_eq!(d.mounts[0].state, MountState::Missing);
    assert_eq!(
        d.mounts[1].browsable.reason.as_ref().unwrap().code,
        NotBrowsableCode::SingleFile
    );
}

#[test]
fn a_service_with_a_container_is_read_from_inspect_not_compose() {
    let seen = [inspected("valheim", true, serde_json::json!([]))];
    let compose = serde_json::json!({
        "name": "valheim",
        "services": {"valheim": {"volumes": [
            {"type": "volume", "source": "old", "target": "/old"},
        ]}},
    });
    let d = discover("valheim", Path::new(DIR), &seen, Some(&compose), &found_all);
    assert!(d.mounts.is_empty());
}

#[test]
fn mounts_reads_the_stacks_containers_and_remembers_the_sources() {
    let a = app();
    a.set_containers(vec![support::container(
        "valheim",
        "valheim",
        "running",
        "Up 1 hour",
    )]);
    a.set_inspect(
        "valheim-valheim-id",
        serde_json::json!({"Mounts": [{
            "Type": "volume", "Name": "valheim_saves", "Source": "/x",
            "Destination": "/config", "RW": true,
        }]}),
    );
    let d = files::mounts(&a.state, &stack()).unwrap();
    assert_eq!(d.mounts.len(), 1);
    let m = &d.mounts[0];
    assert_eq!(m.running, vec!["valheim".to_owned()]);
    let (known, src) = a.state.files.known(&stack(), &m.id).unwrap();
    assert_eq!(&known, m);
    assert_eq!(src, HelperSource::Volume("valheim_saves".into()));
}

// ---------------------------------------------------------------------------
// The helper image and openat2 (T019)
// ---------------------------------------------------------------------------

const IMAGE: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn the_configured_image_wins_and_must_be_an_id() {
    let a = app();
    let id = files::helper_image(Some(IMAGE), Some("ignored"), &*a.runtime).unwrap();
    assert_eq!(id.as_str(), IMAGE);
    let err = files::helper_image(Some("depths:latest"), None, &*a.runtime).unwrap_err();
    assert!(err.starts_with(files::NO_IMAGE), "{err}");
}

#[test]
fn without_one_the_image_is_this_containers_own() {
    let a = app();
    // `HOSTNAME` in a container is its short id; the daemon resolves it.
    a.set_inspect("abc123", serde_json::json!({"Image": IMAGE}));
    let id = files::helper_image(None, Some("abc123"), &*a.runtime).unwrap();
    assert_eq!(id.as_str(), IMAGE);
}

#[test]
fn no_image_anywhere_is_no_helper_image() {
    let a = app();
    for hostname in [None, Some(""), Some("not-a-container")] {
        let err = files::helper_image(None, hostname, &*a.runtime).unwrap_err();
        assert_eq!(err, files::NO_IMAGE, "{hostname:?}");
    }
    // A container whose image is a name, not an id, is no use either.
    a.set_inspect("named", serde_json::json!({"Image": "depths:latest"}));
    let err = files::helper_image(None, Some("named"), &*a.runtime).unwrap_err();
    assert_eq!(err, files::NO_IMAGE);
}

#[test]
fn a_helper_without_openat2_turns_the_feature_off() {
    let a = app();
    let hello = |openat2| depths_volumes::op::Hello {
        proto: 1,
        openat2,
        uid: 1000,
        gid: 1000,
    };
    assert!(a.state.files.check_hello(&hello(true)).is_ok());
    assert_eq!(a.state.files.mode(), FilesMode::Full);
    let refusal = a.state.files.check_hello(&hello(false)).unwrap_err();
    assert_eq!(files::refusal_code(&refusal), "helper_unavailable");
    assert_eq!(a.state.files.mode(), FilesMode::Off);
    assert_eq!(a.state.files.reason().as_deref(), Some(files::NO_OPENAT2));
}

// ---------------------------------------------------------------------------
// Browse, read and download through a fake helper (T030)
// ---------------------------------------------------------------------------

#[path = "../../volume-helper/tests/cases/mod.rs"]
mod cases;

mod browse {
    use super::*;

    use std::os::unix::fs::MetadataExt;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use depths_core::ports::{HelperSpec, OpMode};
    use std::time::Duration;

    use depths_protocol::{
        BinaryReason, CreateKind, FileOp, FilePreview, LineEnding, MountId, Name, PreviewId,
        PreviewVoidReason, VersionReason,
    };
    use http::{StatusCode, header};

    use super::cases::{Access, Expect, build_tree, cases, snapshot};
    use super::support::ws;

    use super::support::volumes::{self, FakeVolumeHelper};

    struct Browse {
        a: App,
        helper: Arc<FakeVolumeHelper>,
        root: PathBuf,
        mount: MountId,
        uid: u32,
        _tree: tempfile::TempDir,
    }

    impl Browse {
        fn specs(&self) -> Vec<HelperSpec> {
            self.helper.specs.lock().unwrap().clone()
        }
        fn read(&self, w: &Who, p: &str) -> files::Answer<depths_protocol::FileContent> {
            files::read_file(&self.a.state, w, &stack(), &self.mount, &rel(p))
        }
        fn list(&self, w: &Who, p: &str) -> files::Answer<Vec<files::Page>> {
            files::list_dir(&self.a.state, w, &stack(), &self.mount, &rel(p))
        }
    }

    fn rel(p: &str) -> RelPath {
        RelPath::parse(p).unwrap()
    }

    /// An app whose `valheim` stack mounts volume `valheim_saves`, backed by the escape tree.
    async fn browse(env: &[(&str, &str)]) -> Browse {
        let tree_dir = tempfile::tempdir().unwrap();
        let tree = build_tree(tree_dir.path());
        std::fs::write(tree.root.join("bin.dat"), b"\x00\x01\x02binary").unwrap();
        let a = app_with_env(env, |_| {});
        a.sign_in(OWNER).await;
        let (helper, _) = volumes::install(&a, tree.root.clone());
        let mounts = files::list_mounts(&a.state, &admin(), &stack()).unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].state, MountState::Ready);
        let uid = std::fs::metadata(&tree.root).unwrap().uid();
        Browse {
            a,
            helper,
            root: tree.root,
            mount: mounts[0].id.clone(),
            uid,
            _tree: tree_dir,
        }
    }

    #[tokio::test]
    async fn list_mounts_dir_pages_and_read_over_the_socket() {
        let b = browse(&[]).await;
        let (mut s, _server, _) = ws::open(&b.a, OWNER).await;
        let r = ws::ask(
            &mut s,
            1,
            serde_json::json!({
                "type": "ListMounts", "data": {"stack": "valheim"},
            }),
        )
        .await;
        assert_eq!(r["event"]["type"], "Mounts", "{r}");
        let m = &r["event"]["data"]["mounts"][0];
        assert_eq!(m["id"], serde_json::json!(b.mount));
        assert_eq!(m["source"], "valheim_saves");
        assert_eq!(m["state"], "ready");

        let r = ws::ask(
            &mut s,
            2,
            serde_json::json!({
                "type": "ListDir",
                "data": {"stack": "valheim", "mount": b.mount, "path": "sub", "req": 41},
            }),
        )
        .await;
        assert_eq!(r["event"]["type"], "DirPage", "{r}");
        let page = &r["event"]["data"];
        assert_eq!(page["req"], 41);
        assert_eq!(page["path"], "sub");
        assert_eq!(page["last"], true);
        let names: Vec<&str> = page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert_eq!(names[0], "deep", "folders first: {names:?}");
        assert!(names.contains(&"b.txt"), "{names:?}");

        let r = ws::ask(
            &mut s,
            3,
            serde_json::json!({
                "type": "ReadFile",
                "data": {"stack": "valheim", "mount": b.mount, "path": "a.txt", "req": 7},
            }),
        )
        .await;
        assert_eq!(r["event"]["type"], "FileContent", "{r}");
        assert_eq!(r["event"]["data"]["req"], 7);
        assert!(r["event"]["data"]["file"]["text"].is_string(), "{r}");
    }

    #[tokio::test]
    async fn a_big_folder_arrives_in_pages() {
        let b = browse(&[]).await;
        let big = b.root.join("big");
        std::fs::create_dir(&big).unwrap();
        for i in 0..5_001 {
            std::fs::write(big.join(format!("f{i:05}")), b"").unwrap();
        }
        let pages = b.list(&admin(), "big").unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries.len(), 5_000);
        assert!(!pages[0].last && pages[1].last);
        assert_eq!((pages[0].page, pages[1].page), (0, 1));
    }

    #[tokio::test]
    async fn read_decodes_text_binary_and_too_large_and_audits_the_read() {
        let b = browse(&[("DEPTHS_FILES_EDITOR_MAX", "64")]).await;
        std::fs::write(b.root.join("big.txt"), "x".repeat(100)).unwrap();

        let text = b.read(&admin(), "sub/b.txt").unwrap();
        assert!(text.text.is_some() && text.binary_reason.is_none());
        let bin = b.read(&admin(), "bin.dat").unwrap();
        assert_eq!(bin.binary_reason, Some(BinaryReason::Binary));
        assert!(bin.text.is_none());
        let big = b.read(&admin(), "big.txt").unwrap();
        assert_eq!(big.binary_reason, Some(BinaryReason::TooLarge));
        assert_eq!(big.size, 100);

        let reads: Vec<_> =
            b.a.audit_entries()
                .into_iter()
                .filter(|e| e.action == AuditAction::FilesRead)
                .collect();
        assert_eq!(reads.len(), 3);
        let place = reads[0].file.as_ref().unwrap();
        assert_eq!(place.source, "valheim_saves");
        assert!(reads.iter().all(|e| e.outcome == AuditOutcome::Succeeded));
        assert!(refused(&b.a).is_empty());
    }

    #[tokio::test]
    async fn the_root_is_stat_ed_as_nobody_and_the_rest_as_its_owner() {
        let b = browse(&[]).await;
        b.list(&admin(), "").unwrap();
        b.read(&admin(), "sub/deep/c.txt").unwrap();
        let specs = b.specs();
        assert_eq!((specs[0].uid, specs[0].gid), files::NOBODY);
        assert!(specs[1..].iter().all(|s| s.uid == b.uid), "{specs:?}");
        assert!(specs.iter().all(|s| s.mode == OpMode::ReadOnly));
        // The listing's owners are reused: reading its children starts no new helper.
        let started = specs.len();
        b.read(&admin(), "a.txt").unwrap();
        assert_eq!(b.specs().len(), started);
    }

    #[tokio::test]
    async fn an_owner_that_moved_is_owner_changed_and_forgotten() {
        let b = browse(&[]).await;
        let p = rel("a.txt");
        b.a.state
            .files
            .note_owners(&stack(), &b.mount, [(p.clone(), (4242, 4242))]);
        let err = *b.read(&admin(), "a.txt").unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::OwnerChanged));
        assert_eq!(b.specs().last().unwrap().uid, 4242);
        assert_eq!(b.a.state.files.owner(&stack(), &b.mount, &p), None);
        assert_eq!(reason_of(&refused(&b.a)[0]), "owner_changed");
        // The next read re-learns the owner and succeeds.
        b.read(&admin(), "a.txt").unwrap();
    }

    #[tokio::test]
    async fn every_refused_escape_case_is_one_files_refused_entry() {
        let b = browse(&[]).await;
        let tree_sentinels = [b.root.parent().unwrap().join("outside/secret.txt")];
        let before = snapshot(&tree_sentinels);
        let mut want = 0;
        for case in cases() {
            // Raw names are not opened in the editor; invalid paths never parse (see below);
            // writes arrive with saving (US2).
            if case.raw_name.is_some()
                || case.expect == Expect::InvalidPath
                || case.access == Access::Write
            {
                continue;
            }
            let path = RelPath::parse(case.path).unwrap();
            let r = files::read_file(&b.a.state, &admin(), &stack(), &b.mount, &path);
            let refused_now = match (&case.expect, case.access, &r) {
                (Expect::OutsideMount, _, Err(e)) => {
                    assert_eq!(
                        **e,
                        Event::FileRefused(FileRefusal::OutsideMount),
                        "{}",
                        case.name
                    );
                    true
                }
                (Expect::Blocked(_), _, Err(e)) => {
                    assert!(
                        matches!(**e, Event::FileRefused(FileRefusal::WriteBlocked { .. })),
                        "{}: {e:?}",
                        case.name
                    );
                    true
                }
                (Expect::NotFound, _, Err(e)) => {
                    assert_eq!(
                        **e,
                        Event::FileRefused(FileRefusal::NotFound),
                        "{}",
                        case.name
                    );
                    true
                }
                // A stat-only case read as a file: whatever it is, it never leaves the root.
                (_, Access::Stat, _) => r.is_err(),
                (Expect::Inside, Access::Read, Ok(f)) => {
                    assert!(
                        !f.text.as_deref().unwrap_or("").contains("SENTINEL"),
                        "{}",
                        case.name
                    );
                    false
                }
                (_, _, r) => panic!("{}: unexpected {r:?}", case.name),
            };
            if refused_now {
                want += 1;
            }
            assert_eq!(refused(&b.a).len(), want, "{}", case.name);
        }
        assert!(want >= 10, "only {want} refused cases ran");
        assert_eq!(snapshot(&tree_sentinels), before);
    }

    #[tokio::test]
    async fn an_invalid_path_is_refused_before_the_gate() {
        let b = browse(&[]).await;
        let (mut s, _server, _) = ws::open(&b.a, OWNER).await;
        for p in ["..", "sub/..", "/etc/passwd"] {
            let r = ws::ask(
                &mut s,
                1,
                serde_json::json!({
                    "type": "ReadFile",
                    "data": {"stack": "valheim", "mount": b.mount, "path": p, "req": 1},
                }),
            )
            .await;
            assert_ne!(r["event"]["type"], "FileContent", "{p}: {r}");
        }
        assert!(b.specs().is_empty());
    }

    #[tokio::test]
    async fn a_viewer_is_refused_and_the_restricted_view_may_list_but_not_read() {
        let b = browse(&[]).await;
        let viewer = who("3003", Role::User, &["other"], support::LOOPBACK);
        let err = *b.read(&viewer, "a.txt").unwrap_err();
        assert!(
            matches!(
                err,
                Event::Denied {
                    reason: DenyReason::NotPermitted,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(files::list_mounts(&b.a.state, &viewer, &stack()).is_err());

        b.a.sign_in_from(AWAY, OWNER).await;
        let away = who(OWNER, Role::Admin, &[], CAFE);
        files::list_mounts(&b.a.state, &away, &stack()).unwrap();
        b.list(&away, "sub").unwrap();
        let err = *b.read(&away, "a.txt").unwrap_err();
        assert!(
            matches!(
                err,
                Event::Denied {
                    reason: DenyReason::UntrustedNetwork,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    fn download_uri(mount: &MountId, path: &str) -> String {
        let q: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("mount", mount.as_str())
            .append_pair("path", path)
            .finish();
        format!("/api/v1/stacks/valheim/files/download?{q}")
    }

    async fn body(r: http::Response<Body>) -> Vec<u8> {
        axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn a_download_streams_the_file_with_safe_headers_and_is_audited() {
        let b = browse(&[("DEPTHS_FILES_DOWNLOAD_MAX", "16")]).await;
        let cookie = b.a.sign_in(OWNER).await;
        std::fs::write(b.root.join("big.txt"), "x".repeat(17)).unwrap();

        let r =
            b.a.get(&download_uri(&b.mount, "sub/b.txt"), Some(&cookie))
                .await;
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers().clone();
        assert_eq!(h[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        assert_eq!(h["x-content-type-options"], "nosniff");
        let disposition = h[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(disposition.starts_with("attachment;"), "{disposition}");
        assert!(disposition.contains("b.txt"), "{disposition}");
        let want = std::fs::read(b.root.join("sub/b.txt")).unwrap();
        assert_eq!(h[header::CONTENT_LENGTH], want.len().to_string().as_str());
        assert_eq!(body(r).await, want);

        let downloads: Vec<_> =
            b.a.audit_entries()
                .into_iter()
                .filter(|e| e.action == AuditAction::FilesDownload)
                .collect();
        assert_eq!(downloads.len(), 1);
        assert_eq!(
            downloads[0].file.as_ref().unwrap().bytes,
            Some(want.len() as u64)
        );

        let r =
            b.a.get(&download_uri(&b.mount, "big.txt"), Some(&cookie))
                .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let r =
            b.a.get(&download_uri(&b.mount, "link_out"), Some(&cookie))
                .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = b.a.get(&download_uri(&b.mount, "a.txt"), None).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // Save (T040)
    // -----------------------------------------------------------------------

    /// Every replaced content handed to history, in order.
    #[derive(Default)]
    struct Kept(Mutex<Vec<Kept1>>);

    /// One kept version: the file, its author, why, and the content.
    type Kept1 = (files::FileKey, String, VersionReason, Vec<u8>);

    impl files::HistorySink for Kept {
        fn keep(&self, file: &files::FileKey, author: &str, reason: VersionReason, bytes: &[u8]) {
            self.0
                .lock()
                .unwrap()
                .push((file.clone(), author.to_owned(), reason, bytes.to_vec()));
        }
    }

    fn save_op(b: &Browse, p: &str, text: &str, f: &depths_protocol::FileContent) -> FileOp {
        FileOp::Save {
            mount: b.mount.clone(),
            path: rel(p),
            text: text.into(),
            base: f.fingerprint,
            bom: f.bom,
            eol: f.save_eol,
        }
    }

    impl Browse {
        fn preview(&self, w: &Who, op: FileOp) -> files::Answer<FilePreview> {
            files::preview(&self.a.state, w, &stack(), op)
        }
        fn confirm(&self, w: &Who, id: PreviewId) -> files::Answer<Event> {
            files::confirm(&self.a.state, w, id)
        }
        fn kept(&self) -> Arc<Kept> {
            let kept = Arc::new(Kept::default());
            self.a.state.files.set_history(kept.clone());
            kept
        }
        fn rw_specs(&self) -> Vec<HelperSpec> {
            self.specs()
                .into_iter()
                .filter(|s| s.mode == OpMode::ReadWrite)
                .collect()
        }
    }

    fn audited(a: &App, action: AuditAction) -> Vec<depths_protocol::AuditEntry> {
        a.audit_entries()
            .into_iter()
            .filter(|e| e.action == action)
            .collect()
    }

    #[tokio::test]
    async fn a_save_previews_a_diff_and_confirm_writes_once_as_the_owner() {
        let b = browse(&[]).await;
        let kept = b.kept();
        let old = std::fs::read(b.root.join("a.txt")).unwrap();
        let f = b.read(&admin(), "a.txt").unwrap();
        let p = b
            .preview(&admin(), save_op(&b, "a.txt", "hello A\n", &f))
            .unwrap();
        let diff = p.diff.as_deref().unwrap();
        assert!(diff.contains("-hello a"), "{diff}");
        assert!(diff.contains("+hello A"), "{diff}");
        assert_eq!(p.running, vec!["valheim".to_owned()]);
        assert!(p.summary.contains("a.txt"), "{}", p.summary);
        assert_eq!(p.stack, "valheim");
        // Nothing is written by the preview, and no read-write helper started.
        assert_eq!(std::fs::read(b.root.join("a.txt")).unwrap(), old);
        assert!(b.rw_specs().is_empty());

        let done = b.confirm(&admin(), p.preview_id).unwrap();
        let Event::FileOpDone {
            preview_id,
            fingerprint,
            running,
            path,
            ..
        } = done
        else {
            panic!("{done:?}")
        };
        assert_eq!(preview_id, p.preview_id);
        assert_eq!(path, rel("a.txt"));
        assert_eq!(running, vec!["valheim".to_owned()]);
        let on_disk = std::fs::read(b.root.join("a.txt")).unwrap();
        assert_eq!(on_disk, b"hello A\n");
        assert_eq!(
            fingerprint.unwrap().sha256_hex(),
            Fingerprint::of_content(&on_disk, 0, 0).sha256_hex()
        );
        let rw = b.rw_specs();
        assert_eq!(rw.len(), 1, "one read-write helper: {rw:?}");
        assert_eq!(rw[0].uid, b.uid);

        let saves = audited(&b.a, AuditAction::FilesSave);
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].outcome, AuditOutcome::Succeeded);
        assert_eq!(saves[0].file.as_ref().unwrap().bytes, Some(8));
        assert_eq!(saves[0].file.as_ref().unwrap().path, rel("a.txt"));

        let got = kept.0.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.path, rel("a.txt"));
        assert_eq!(got[0].1, format!("discord:{OWNER}"));
        assert_eq!(got[0].2, VersionReason::Save);
        assert_eq!(got[0].3, old);

        // A preview confirms once.
        let again = *b.confirm(&admin(), p.preview_id).unwrap_err();
        assert_eq!(
            again,
            Event::PreviewVoid {
                reason: PreviewVoidReason::Used
            }
        );
    }

    #[tokio::test]
    async fn a_save_over_the_socket_is_previewed_then_confirmed() {
        let b = browse(&[]).await;
        let f = b.read(&admin(), "sub/b.txt").unwrap();
        let (mut s, _server, _) = ws::open(&b.a, OWNER).await;
        let op = serde_json::to_value(save_op(&b, "sub/b.txt", "bye b", &f)).unwrap();
        let r = ws::ask(
            &mut s,
            1,
            serde_json::json!({"type": "PreviewFileOp", "data": {"stack": "valheim", "op": op}}),
        )
        .await;
        assert_eq!(r["event"]["type"], "FilePreview", "{r}");
        let id = r["event"]["data"]["preview_id"].clone();
        let r = ws::ask(
            &mut s,
            2,
            serde_json::json!({"type": "Confirm", "data": {"preview_id": id}}),
        )
        .await;
        assert_eq!(r["event"]["type"], "FileOpDone", "{r}");
        assert!(r["event"]["data"]["fingerprint"].is_string(), "{r}");
        assert_eq!(std::fs::read(b.root.join("sub/b.txt")).unwrap(), b"bye b");
    }

    #[tokio::test]
    async fn read_only_mode_and_a_read_only_mount_refuse_the_save() {
        let b = browse(&[("DEPTHS_FEATURE_FILES", "read_only")]).await;
        let f = b.read(&admin(), "a.txt").unwrap();
        let err = *b
            .preview(&admin(), save_op(&b, "a.txt", "x", &f))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::FeatureReadOnly));

        let b = browse(&[]).await;
        b.a.set_inspect(
            "valheim-valheim-id",
            serde_json::json!({"Mounts": [{
                "Type": "volume", "Name": "valheim_saves", "Source": "/x",
                "Destination": "/config", "RW": false,
            }]}),
        );
        files::list_mounts(&b.a.state, &admin(), &stack()).unwrap();
        let f = b.read(&admin(), "a.txt").unwrap();
        let err = *b
            .preview(&admin(), save_op(&b, "a.txt", "x", &f))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::ReadOnlyMount));
        assert!(b.rw_specs().is_empty());
        assert_eq!(reason_of(refused(&b.a).last().unwrap()), "read_only_mount");
    }

    #[tokio::test]
    async fn crlf_and_bom_round_trip_byte_exact_when_one_line_changed() {
        let b = browse(&[]).await;
        let before = b"\xEF\xBB\xBFone\r\ntwo\r\nthree\r\n";
        std::fs::write(b.root.join("w.ini"), before).unwrap();
        let f = b.read(&admin(), "w.ini").unwrap();
        assert!(f.bom);
        assert_eq!(f.save_eol, LineEnding::Crlf);
        let text = f.text.clone().unwrap().replace("two", "TWO");
        let p = b
            .preview(&admin(), save_op(&b, "w.ini", &text, &f))
            .unwrap();
        let diff = p.diff.unwrap();
        let removed = diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        assert_eq!(removed, 1, "{diff}");
        b.confirm(&admin(), p.preview_id).unwrap();
        assert_eq!(
            std::fs::read(b.root.join("w.ini")).unwrap(),
            b"\xEF\xBB\xBFone\r\nTWO\r\nthree\r\n"
        );
    }

    #[tokio::test]
    async fn a_save_over_a_changed_file_is_changed_on_disk() {
        let b = browse(&[]).await;
        let f = b.read(&admin(), "a.txt").unwrap();
        std::fs::write(b.root.join("a.txt"), "someone else").unwrap();
        let err = *b
            .preview(&admin(), save_op(&b, "a.txt", "mine", &f))
            .unwrap_err();
        assert!(
            matches!(err, Event::FileRefused(FileRefusal::ChangedOnDisk { .. })),
            "{err:?}"
        );

        // Changed between the preview and the confirm: the helper's base check refuses it.
        let f = b.read(&admin(), "a.txt").unwrap();
        let p = b
            .preview(&admin(), save_op(&b, "a.txt", "mine", &f))
            .unwrap();
        std::fs::write(b.root.join("a.txt"), "someone else again").unwrap();
        let err = *b.confirm(&admin(), p.preview_id).unwrap_err();
        assert!(
            matches!(err, Event::FileRefused(FileRefusal::ChangedOnDisk { .. })),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read(b.root.join("a.txt")).unwrap(),
            b"someone else again"
        );
        assert!(audited(&b.a, AuditAction::FilesSave).is_empty());
    }

    #[tokio::test]
    async fn a_preview_expires_and_belongs_to_its_session() {
        let b = browse(&[]).await;
        let f = b.read(&admin(), "a.txt").unwrap();
        let old = std::fs::read(b.root.join("a.txt")).unwrap();

        let p = b.preview(&admin(), save_op(&b, "a.txt", "x", &f)).unwrap();
        let mut other = admin();
        other.session = "another-session".into();
        let err = *b.confirm(&other, p.preview_id).unwrap_err();
        assert_eq!(
            err,
            Event::PreviewVoid {
                reason: PreviewVoidReason::Unknown
            }
        );
        assert!(!b.a.state.files.has_preview(&other.session, p.preview_id));
        assert!(b.a.state.files.has_preview("test-session", p.preview_id));

        b.a.state.files.set_preview_ttl(Duration::ZERO);
        let p = b.preview(&admin(), save_op(&b, "a.txt", "x", &f)).unwrap();
        let err = *b.confirm(&admin(), p.preview_id).unwrap_err();
        assert_eq!(
            err,
            Event::PreviewVoid {
                reason: PreviewVoidReason::Expired
            }
        );
        assert_eq!(std::fs::read(b.root.join("a.txt")).unwrap(), old);
        assert!(b.rw_specs().is_empty());
    }

    #[tokio::test]
    async fn the_default_preview_lifetime_is_five_minutes() {
        let b = browse(&[]).await;
        let f = b.read(&admin(), "a.txt").unwrap();
        let p = b.preview(&admin(), save_op(&b, "a.txt", "x", &f)).unwrap();
        let at = chrono::DateTime::parse_from_rfc3339(&p.expires_at).unwrap();
        let left = at.signed_duration_since(chrono::Utc::now()).num_seconds();
        assert!((295..=300).contains(&left), "{left}");
    }

    // -----------------------------------------------------------------------
    // Create, rename and delete (T053)
    // -----------------------------------------------------------------------

    fn create_op(b: &Browse, parent: &str, name: &str, kind: CreateKind) -> FileOp {
        FileOp::Create {
            mount: b.mount.clone(),
            parent: rel(parent),
            name: Name::new(name).unwrap(),
            kind,
        }
    }

    fn rename_op(b: &Browse, from: &str, to: &str) -> FileOp {
        FileOp::Rename {
            mount: b.mount.clone(),
            from: rel(from),
            to: rel(to),
        }
    }

    fn delete_op(b: &Browse, p: &str) -> FileOp {
        FileOp::Delete {
            mount: b.mount.clone(),
            path: rel(p),
            raw_name: None,
        }
    }

    /// Fakes `path`'s owner, as a listing would have recorded it.
    fn own(b: &Browse, path: &str, owner: (u32, u32)) {
        b.a.state
            .files
            .note_owners(&stack(), &b.mount, [(rel(path), owner)]);
    }

    fn done_path(e: &Event) -> RelPath {
        match e {
            Event::FileOpDone { path, .. } => path.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn create_previews_the_full_path_and_runs_as_the_parents_owner() {
        let b = browse(&[]).await;
        own(&b, "sub", (4242, 4243));
        let p = b
            .preview(&admin(), create_op(&b, "sub", "new.txt", CreateKind::File))
            .unwrap();
        assert!(p.summary.contains("sub/new.txt"), "{}", p.summary);
        assert!(p.summary.contains("file"), "{}", p.summary);
        assert!(!b.root.join("sub/new.txt").exists());
        assert!(b.rw_specs().is_empty());

        let done = b.confirm(&admin(), p.preview_id).unwrap();
        assert_eq!(done_path(&done), rel("sub/new.txt"));
        assert_eq!(std::fs::read(b.root.join("sub/new.txt")).unwrap(), b"");
        let rw = b.rw_specs();
        assert_eq!(rw.len(), 1, "{rw:?}");
        assert_eq!((rw[0].uid, rw[0].gid), (4242, 4243));

        let p = b
            .preview(&admin(), create_op(&b, "", "newdir", CreateKind::Dir))
            .unwrap();
        assert!(p.summary.contains("folder"), "{}", p.summary);
        b.confirm(&admin(), p.preview_id).unwrap();
        assert!(b.root.join("newdir").is_dir());

        let creates = audited(&b.a, AuditAction::FilesCreate);
        assert_eq!(creates.len(), 2);
        let paths: Vec<_> = creates
            .iter()
            .map(|e| e.file.clone().unwrap().path)
            .collect();
        assert!(paths.contains(&rel("sub/new.txt")), "{paths:?}");
        assert!(creates.iter().all(|e| e.outcome == AuditOutcome::Succeeded));
    }

    #[tokio::test]
    async fn a_name_that_exists_or_the_mount_root_is_refused_at_the_preview() {
        let b = browse(&[]).await;
        let err = *b
            .preview(&admin(), create_op(&b, "", "a.txt", CreateKind::File))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::Exists));
        let err = *b
            .preview(&admin(), rename_op(&b, "a.txt", "sub/b.txt"))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::Exists));
        let err = *b
            .preview(&admin(), rename_op(&b, "nope.txt", "nope2.txt"))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::NotFound));
        let err = *b.preview(&admin(), delete_op(&b, "")).unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::InvalidName));
        assert!(b.rw_specs().is_empty());
        assert_eq!(reason_of(refused(&b.a).last().unwrap()), "invalid_name");
    }

    #[tokio::test]
    async fn a_rename_names_old_and_new_and_is_audited() {
        let b = browse(&[]).await;
        let p = b
            .preview(&admin(), rename_op(&b, "sub/b.txt", "sub/deep/b2.txt"))
            .unwrap();
        assert!(
            p.summary.contains("sub/b.txt → sub/deep/b2.txt"),
            "{}",
            p.summary
        );
        assert!(b.root.join("sub/b.txt").exists());
        let done = b.confirm(&admin(), p.preview_id).unwrap();
        assert_eq!(done_path(&done), rel("sub/deep/b2.txt"));
        assert!(!b.root.join("sub/b.txt").exists());
        assert_eq!(
            std::fs::read(b.root.join("sub/deep/b2.txt")).unwrap(),
            b"hello b"
        );
        let rw = b.rw_specs();
        assert_eq!(rw.len(), 1);
        assert_eq!(rw[0].uid, b.uid);
        let renames = audited(&b.a, AuditAction::FilesRename);
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].file.as_ref().unwrap().path, rel("sub/b.txt"));
    }

    #[tokio::test]
    async fn a_rename_between_folders_of_different_owners_is_refused_before_any_write() {
        let b = browse(&[]).await;
        own(&b, "sub", (4242, 4242));
        let err = *b
            .preview(&admin(), rename_op(&b, "a.txt", "sub/a.txt"))
            .unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::MixedOwners));
        assert!(b.rw_specs().is_empty());
        assert_eq!(reason_of(refused(&b.a).last().unwrap()), "mixed_owners");

        // The owners moved apart between the preview and the confirm.
        let gid = std::fs::metadata(&b.root).unwrap().gid();
        own(&b, "sub", (b.uid, gid));
        let p = b
            .preview(&admin(), rename_op(&b, "a.txt", "sub/a.txt"))
            .unwrap();
        own(&b, "sub", (4242, 4242));
        let err = *b.confirm(&admin(), p.preview_id).unwrap_err();
        assert_eq!(err, Event::FileRefused(FileRefusal::MixedOwners));
        assert!(b.rw_specs().is_empty());
        assert!(b.root.join("a.txt").exists());
    }

    #[tokio::test]
    async fn a_file_delete_keeps_its_content_in_history() {
        let b = browse(&[]).await;
        let kept = b.kept();
        let p = b
            .preview(&admin(), delete_op(&b, "sub/deep/c.txt"))
            .unwrap();
        assert!(p.summary.contains("sub/deep/c.txt"), "{}", p.summary);
        assert_eq!(p.entries, None);
        assert!(b.root.join("sub/deep/c.txt").exists());

        b.confirm(&admin(), p.preview_id).unwrap();
        assert!(!b.root.join("sub/deep/c.txt").exists());
        let rw = b.rw_specs();
        assert_eq!(rw.len(), 1);
        assert_eq!(rw[0].uid, b.uid);
        let deletes = audited(&b.a, AuditAction::FilesDelete);
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].outcome, AuditOutcome::Succeeded);
        assert_eq!(deletes[0].file.as_ref().unwrap().bytes, Some(7));

        let got = kept.0.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.path, rel("sub/deep/c.txt"));
        assert_eq!(got[0].2, VersionReason::Delete);
        assert_eq!(got[0].3, b"hello c");
    }

    #[tokio::test]
    async fn a_folder_delete_counts_its_entries_up_to_ten_thousand() {
        let b = browse(&[]).await;
        let kept = b.kept();
        let p = b.preview(&admin(), delete_op(&b, "sub")).unwrap();
        assert!(!p.entries_capped);
        assert!(p.entries.unwrap() >= 3, "{p:?}");
        assert!(p.summary.contains("sub"), "{}", p.summary);

        let many = b.root.join("many");
        std::fs::create_dir(&many).unwrap();
        for i in 0..10_001 {
            std::fs::write(many.join(format!("f{i:05}")), b"").unwrap();
        }
        let p = b.preview(&admin(), delete_op(&b, "many")).unwrap();
        assert_eq!(p.entries, Some(10_000));
        assert!(p.entries_capped);
        assert!(p.summary.contains("10,000+"), "{}", p.summary);
        assert!(many.exists());

        b.confirm(&admin(), p.preview_id).unwrap();
        assert!(!many.exists());
        assert_eq!(audited(&b.a, AuditAction::FilesDelete).len(), 1);
        assert!(
            kept.0.lock().unwrap().is_empty(),
            "a folder keeps no history"
        );
    }

    #[tokio::test]
    async fn a_partial_delete_is_audited_with_what_was_removed_and_shown_verbatim() {
        let b = browse(&[]).await;
        if b.uid == 0 {
            eprintln!("skipped: root removes entries from a read-only folder anyway");
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let d = b.root.join("d");
        std::fs::create_dir_all(d.join("locked")).unwrap();
        std::fs::write(d.join("a"), b"a").unwrap();
        std::fs::write(d.join("locked/f"), b"f").unwrap();
        std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let p = b.preview(&admin(), delete_op(&b, "d")).unwrap();
        let err = *b.confirm(&admin(), p.preview_id).unwrap_err();
        std::fs::set_permissions(d.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let Event::FileRefused(FileRefusal::Partial {
            removed,
            failed_at,
            errno,
            message,
        }) = err
        else {
            panic!("{err:?}")
        };
        assert_eq!(removed, 1);
        assert_eq!(failed_at, "d/locked/f");
        assert_eq!(errno, "EACCES");
        assert!(!message.is_empty());

        let deletes = audited(&b.a, AuditAction::FilesDelete);
        assert_eq!(deletes.len(), 1, "{deletes:?}");
        let AuditOutcome::Failed { stderr_tail, .. } = &deletes[0].outcome else {
            panic!("{deletes:?}")
        };
        assert!(stderr_tail.contains("removed 1"), "{stderr_tail}");
        assert!(stderr_tail.contains("d/locked/f"), "{stderr_tail}");
        assert!(stderr_tail.contains(&message), "{stderr_tail}");
    }

    #[tokio::test]
    async fn a_rename_over_the_socket_is_previewed_then_confirmed() {
        let b = browse(&[]).await;
        let (mut s, _server, _) = ws::open(&b.a, OWNER).await;
        let op = serde_json::to_value(rename_op(&b, "a.txt", "a2.txt")).unwrap();
        let r = ws::ask(
            &mut s,
            1,
            serde_json::json!({"type": "PreviewFileOp", "data": {"stack": "valheim", "op": op}}),
        )
        .await;
        assert_eq!(r["event"]["type"], "FilePreview", "{r}");
        let id = r["event"]["data"]["preview_id"].clone();
        let r = ws::ask(
            &mut s,
            2,
            serde_json::json!({"type": "Confirm", "data": {"preview_id": id}}),
        )
        .await;
        assert_eq!(r["event"]["type"], "FileOpDone", "{r}");
        assert!(r["event"]["data"]["fingerprint"].is_null(), "{r}");
        assert!(b.root.join("a2.txt").exists());
    }
}
