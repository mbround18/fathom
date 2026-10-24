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
