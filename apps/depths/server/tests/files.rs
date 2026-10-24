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
