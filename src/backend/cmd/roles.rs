//! Role assignment commands (033 US4): preview rendering and orphaned-subject detection.
//! Both are read-only; the write path is the existing `ApplyResourceYaml`.

use crossbeam_channel::Sender;
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding, Subject};
use kube::{
    ResourceExt,
    api::{Api, ListParams},
};

use crate::backend::events;
use crate::backend::persistence;
use crate::backend::role_assignment::{self, BindingSubject};
use crate::backend::state::BackendState;
use crate::protocol::v1::IdpDirectorySnapshot;
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Handles the role-assignment commands; returns the command back if it belongs elsewhere.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::PreviewRoleAssignment { draft } => {
            let config = persistence::load_oidc_config();
            let Some(issuer) = config.issuers.iter().find(|i| i.id == draft.issuer_id) else {
                emit(
                    state,
                    sender,
                    SystemEvent::OidcError {
                        issuer_id: draft.issuer_id.clone(),
                        stage: "role-assignment".into(),
                        message: "unknown issuer".into(),
                    },
                );
                return Ok(None);
            };
            let p = role_assignment::preview(issuer, &draft);
            emit(
                state,
                sender,
                SystemEvent::RoleAssignmentPreview {
                    yaml: p.yaml,
                    rendered_subject_name: p.rendered_subject_name,
                    warnings: p.warnings,
                },
            );
            Ok(None)
        }
        BackendCmd::RefreshOrphanedSubjects { issuer_id, .. } => {
            let config = persistence::load_oidc_config();
            let orphans = match (
                config.issuers.iter().find(|i| i.id == issuer_id),
                directory_for(state, &issuer_id),
            ) {
                (Some(issuer), Some(dir)) => {
                    let subjects = list_subjects(state).await?;
                    role_assignment::find_orphans(issuer, &dir, &subjects)
                }
                _ => Vec::new(),
            };
            emit(
                state,
                sender,
                SystemEvent::OrphanedSubjectsUpdated { orphans },
            );
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

/// The cached IdP directory for an issuer. `None` when it was never fetched, in which case
/// there is no evidence of absence and orphan detection reports nothing.
fn directory_for(state: &BackendState, issuer_id: &str) -> Option<IdpDirectorySnapshot> {
    state.idp.snapshot(issuer_id)
}

fn emit(state: &BackendState, sender: &Sender<BackendEventEnvelope>, event: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(event),
    );
}

fn collect(out: &mut Vec<BindingSubject>, binding: &str, subjects: Option<Vec<Subject>>) {
    for s in subjects.unwrap_or_default() {
        out.push(BindingSubject {
            binding: binding.to_owned(),
            kind: s.kind,
            name: s.name,
        });
    }
}

async fn list_subjects(state: &BackendState) -> anyhow::Result<Vec<BindingSubject>> {
    let client = state.cluster.client_or_err()?;
    let mut out = Vec::new();
    let rbs = Api::<RoleBinding>::all(client.clone())
        .list(&ListParams::default())
        .await?;
    for rb in rbs.items {
        let id = format!(
            "RoleBinding/{}/{}",
            rb.namespace().unwrap_or_default(),
            rb.name_any()
        );
        collect(&mut out, &id, rb.subjects);
    }
    let crbs = Api::<ClusterRoleBinding>::all(client)
        .list(&ListParams::default())
        .await?;
    for crb in crbs.items {
        let id = format!("ClusterRoleBinding/{}", crb.name_any());
        collect(&mut out, &id, crb.subjects);
    }
    Ok(out)
}
