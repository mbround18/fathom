//! Fathom's side of the adoption gate (048): which of its commands need which grant, and where its
//! grants live. The store and the gate itself are the shared runtime crate's.

use crate::protocol::v1::BackendCmdV1;

pub use ::container_runtime::adoption::*;

use super::super::persistence::fathom_config_dir;

/// Fathom's grants, from `<config dir>/container_adoption.json` (the path 048 shipped with,
/// unchanged per Principle VI). No config dir is a fallback: nothing adopted, and the operator is
/// told.
pub fn load() -> Loaded {
    match fathom_config_dir() {
        Ok(dir) => AdoptionStore::open(dir.join("container_adoption.json")),
        Err(_) => Loaded {
            store: AdoptionStore::default(),
            fell_back: true,
        },
    }
}

/// What a command needs, by command.
///
/// Every container-runtime command is listed explicitly. The compiler cannot force that — a
/// `BackendCmdV1` match here would have to enumerate the whole protocol — so
/// [`tests::every_container_runtime_command_is_classified`] does, against the same command list
/// the hosted-policy test uses. A new command added without a line here fails that test.
#[allow(clippy::match_same_arms)]
pub fn requirement(cmd: &BackendCmdV1) -> Requirement {
    use BackendCmdV1 as C;
    match cmd {
        // Reading. Available whether or not anything is adopted — that is the point.
        C::RefreshContainerRuntimePresence
        | C::RefreshContainerInventory { .. }
        | C::LoadProjectDefinition { .. }
        | C::StartContainerUsageSampling { .. }
        | C::StopContainerUsageSampling => Requirement::Free,

        // Previewing derives an argv and a removal set and runs nothing, so it is readable for
        // any project. A preview of an unadopted project is how an operator decides whether to
        // adopt it; refusing here would make adoption a blind choice.
        C::PreviewContainerOperation { .. } => Requirement::Free,

        // Granting and revoking the grant itself. Gating these on the grant would be circular.
        C::AdoptContainerProject { .. } | C::ReleaseContainerProject { .. } => Requirement::Free,

        // Logs are a read. Stopping a stream Fathom started is bookkeeping, not a mutation.
        C::StartContainerLogStream { container_id, .. } => {
            Requirement::Container(container_id.clone())
        }
        C::StopContainerLogStream { .. } | C::StopContainerSession { .. } => Requirement::Free,

        // An interactive session inside a container is a mutation path by any other name
        // (FR-013): whatever the operator types in it runs with the container's privileges.
        C::StartContainerSession { container_id, .. } => {
            Requirement::Container(container_id.clone())
        }
        C::SendContainerSessionInput { .. } | C::ResizeContainerSession { .. } => {
            // The gate already ran when the session was opened. Re-deriving it per keystroke
            // would mean a release mid-session kills the shell under the operator's hands with
            // no explanation; closing the session is the honest way to revoke it.
            Requirement::Free
        }

        C::IssueContainerOperation { .. } => Requirement::FromPreview,

        _ => Requirement::OutOfScope,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::{
        LifecycleAction, OperationRefusal, OperationRequest, OperationScope, StreamId,
    };

    struct Owned {
        project: Option<&'static str>,
        cluster: Option<&'static str>,
    }

    impl Ownership for Owned {
        fn project_of(&self, _: &str) -> Option<String> {
            self.project.map(str::to_owned)
        }
        fn managed_cluster_of(&self, _: &str) -> Option<String> {
            self.cluster.map(str::to_owned)
        }
    }

    const NOTHING: Owned = Owned {
        project: None,
        cluster: None,
    };

    /// Every command this feature adds, in the same order and with the same shapes as the
    /// hosted-policy test. The gate must have an opinion about each one.
    fn every_container_command() -> Vec<BackendCmdV1> {
        use BackendCmdV1 as C;
        let sid = StreamId(1 << 32);
        vec![
            C::RefreshContainerRuntimePresence,
            C::RefreshContainerInventory { classes: vec![] },
            C::LoadProjectDefinition {
                project: "alpha".into(),
            },
            C::AdoptContainerProject {
                project: "alpha".into(),
            },
            C::ReleaseContainerProject {
                project: "alpha".into(),
            },
            C::PreviewContainerOperation {
                request_id: "r1".into(),
                request: OperationRequest::Lifecycle {
                    action: LifecycleAction::Stop,
                    scope: OperationScope::Container { id: "cid".into() },
                },
            },
            C::IssueContainerOperation {
                request_id: "r1".into(),
                preview_id: "p1".into(),
                typed_confirmation: None,
            },
            C::StartContainerLogStream {
                stream_id: sid,
                container_id: "cid".into(),
                follow: true,
                tail_lines: Some(200),
            },
            C::StopContainerLogStream { stream_id: sid },
            C::StartContainerSession {
                stream_id: sid,
                container_id: "cid".into(),
                shell: None,
            },
            C::StopContainerSession { stream_id: sid },
            C::SendContainerSessionInput {
                stream_id: sid,
                data: "ls\n".into(),
            },
            C::ResizeContainerSession {
                stream_id: sid,
                cols: 120,
                rows: 40,
            },
            C::StartContainerUsageSampling {
                container_ids: vec!["cid".into()],
            },
            C::StopContainerUsageSampling,
        ]
    }

    #[test]
    fn every_container_runtime_command_is_classified() {
        for cmd in every_container_command() {
            assert_ne!(
                requirement(&cmd),
                Requirement::OutOfScope,
                "{cmd:?} has no line in requirement(); a new command must be classified"
            );
        }
    }

    /// The whole premise: nothing is actionable until the operator says so.
    #[test]
    fn an_empty_store_refuses_every_gated_command() {
        let store = AdoptionStore::default();
        let owner = Owned {
            project: Some("alpha"),
            cluster: None,
        };
        let gated: Vec<Requirement> = every_container_command()
            .iter()
            .map(requirement)
            .filter(|r| *r != Requirement::Free)
            .collect();
        assert!(!gated.is_empty(), "something has to be gated");
        for req in gated {
            assert!(
                store.check(&req, &owner).is_err(),
                "{req:?} was permitted with nothing adopted"
            );
        }
    }

    /// Reading is not gated. An operator who cannot see a project cannot decide to adopt it.
    #[test]
    fn reads_and_previews_work_with_nothing_adopted() {
        let store = AdoptionStore::default();
        for cmd in every_container_command() {
            let req = requirement(&cmd);
            if matches!(
                cmd,
                BackendCmdV1::RefreshContainerRuntimePresence
                    | BackendCmdV1::RefreshContainerInventory { .. }
                    | BackendCmdV1::LoadProjectDefinition { .. }
                    | BackendCmdV1::PreviewContainerOperation { .. }
            ) {
                assert_eq!(req, Requirement::Free, "{cmd:?}");
                assert!(store.check(&req, &NOTHING).is_ok());
            }
        }
    }

    /// An exec session is a mutation path: whatever is typed into it runs in the container.
    #[test]
    fn an_exec_session_needs_adoption_but_logs_of_the_same_container_do_too() {
        let session = requirement(&BackendCmdV1::StartContainerSession {
            stream_id: StreamId(1 << 32),
            container_id: "cid".into(),
            shell: None,
        });
        assert_eq!(session, Requirement::Container("cid".into()));
        let store = AdoptionStore::default();
        let owner = Owned {
            project: Some("slam-usa"),
            cluster: None,
        };
        assert!(store.check(&session, &owner).is_err());
    }

    /// Issuing names a preview, not a target, so the gate resolves the preview's scope first
    /// (the crate tests that an unresolved `FromPreview` is refused).
    #[test]
    fn issuing_is_gated_through_its_preview() {
        let req = requirement(&BackendCmdV1::IssueContainerOperation {
            request_id: "r1".into(),
            preview_id: "p1".into(),
            typed_confirmation: None,
        });
        assert_eq!(req, Requirement::FromPreview);
        assert_eq!(
            AdoptionStore::default().check(&req, &NOTHING),
            Err(OperationRefusal::PreviewStale)
        );
    }

    // Persistence: Fathom's grants are read from the path it has always used.
    #[test]
    fn fathom_reads_its_grants_from_its_config_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_path_buf();
        std::thread::spawn(move || {
            crate::backend::persistence::set_thread_state_root(root.clone());
            let path = fathom_config_dir().unwrap().join("container_adoption.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, r#"{"projects": ["alpha"]}"#).unwrap();
            let loaded = load();
            assert!(!loaded.fell_back);
            assert_eq!(loaded.store.adopted(), vec!["alpha".to_owned()]);
        })
        .join()
        .expect("scoped thread");
    }
}
