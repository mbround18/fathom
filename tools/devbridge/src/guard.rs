//! The write guard: which commands the dev bridge forwards to the backend.

use fathom::protocol::{BackendCmd, MutationTarget};

/// Confines cluster writes to one context and one sandbox namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGuard {
    pub context: String,
    pub write_namespace: String,
}

impl WriteGuard {
    pub fn new(context: impl Into<String>, write_namespace: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            write_namespace: write_namespace.into(),
        }
    }

    /// `Ok` if `cmd` may be forwarded, otherwise the reason it is refused.
    ///
    /// - `ReadOnly` and `Local` pass, with one exception: `SetMcpServerEnabled` is refused
    ///   outright. It classifies as `Local` (it writes a preference, not the cluster), but a
    ///   dev/debug bridge must never be able to expose an MCP endpoint. The backend also
    ///   hard-sets `Args::disable_mcp` in a devbridge shell, so this is defence in depth: the
    ///   command never even reaches it.
    /// - `Namespaced` passes only if every target namespace is the write namespace.
    /// - `ClusterScoped` never passes.
    /// - `ContextSwitch` passes only to the guarded context.
    pub fn check(&self, cmd: &BackendCmd) -> Result<(), String> {
        if matches!(cmd, BackendCmd::SetMcpServerEnabled { .. }) {
            return Err(
                "SetMcpServerEnabled is refused; a dev bridge must never toggle the MCP server"
                    .to_owned(),
            );
        }
        match cmd.mutation_target() {
            MutationTarget::ReadOnly | MutationTarget::Local => Ok(()),
            MutationTarget::ClusterScoped => Err(format!(
                "{} writes a cluster-scoped resource; only namespace '{}' is writable",
                variant_name(cmd),
                self.write_namespace
            )),
            MutationTarget::Namespaced { namespaces } => {
                let outside: Vec<&str> = namespaces
                    .iter()
                    .map(String::as_str)
                    .filter(|ns| *ns != self.write_namespace)
                    .collect();
                if outside.is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "{} writes outside the sandbox namespace '{}' (targets: {})",
                        variant_name(cmd),
                        self.write_namespace,
                        outside.join(", ")
                    ))
                }
            }
            MutationTarget::ContextSwitch { context } if context == self.context => Ok(()),
            MutationTarget::ContextSwitch { context } => Err(format!(
                "SetContext to '{context}' refused; this bridge is pinned to context '{}'",
                self.context
            )),
        }
    }
}

/// The command's wire tag (e.g. `DeletePod`), for log and error messages.
fn variant_name(cmd: &BackendCmd) -> String {
    serde_json::to_value(cmd)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "command".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> WriteGuard {
        WriteGuard::new("kind-dev", "fathom-dev")
    }

    fn s(v: &str) -> String {
        v.to_owned()
    }

    /// 015-mcp-agent-exposure: the toggle that would start the stdio MCP server is refused
    /// even though `mutation_target()` classifies it as `Local`. Restores the guarantee the
    /// deleted `mcp_server_cannot_be_started` test held for `StartMcpServer`.
    #[test]
    fn mcp_server_cannot_be_enabled() {
        for enabled in [true, false] {
            let err = guard()
                .check(&BackendCmd::SetMcpServerEnabled { enabled })
                .expect_err("the dev bridge must never toggle the MCP server");
            assert!(err.contains("SetMcpServerEnabled"), "{err}");
        }
        // The status read is harmless and still passes.
        assert!(guard().check(&BackendCmd::LoadMcpServerStatus).is_ok());
    }

    #[test]
    fn read_only_and_local_pass() {
        let g = guard();
        for cmd in [
            BackendCmd::Initialize,
            BackendCmd::RefreshPods,
            BackendCmd::SelectPod {
                namespace: s("kube-system"),
                name: s("coredns"),
            },
            BackendCmd::RefreshClusterRoles,
            BackendCmd::LoadUiConfig,
            BackendCmd::StartPortForward {
                namespace: s("kube-system"),
                pod: s("p"),
                local_port: 1,
                remote_port: 2,
                container: None,
            },
        ] {
            assert_eq!(g.check(&cmd), Ok(()), "{cmd:?}");
        }
    }

    #[test]
    fn namespaced_writes_only_inside_the_sandbox() {
        let g = guard();
        let inside = BackendCmd::ScaleDeployment {
            namespace: s("fathom-dev"),
            name: s("scale-me"),
            replicas: 2,
        };
        assert_eq!(g.check(&inside), Ok(()));

        let outside = BackendCmd::DeletePod {
            namespace: s("default"),
            name: s("web"),
        };
        let err = g.check(&outside).unwrap_err();
        assert!(
            err.contains("DeletePod") && err.contains("default"),
            "{err}"
        );
    }

    /// 024-pod-exec-terminal: opening an exec session is gated exactly like any other
    /// namespaced write — a dev bridge sandboxed to `fathom-dev` must not forward
    /// `StartPodExec` against another namespace.
    #[test]
    fn start_pod_exec_is_gated_like_a_namespaced_write() {
        let g = guard();
        let inside = BackendCmd::StartPodExec {
            namespace: s("fathom-dev"),
            name: s("web-0"),
            container: None,
        };
        assert_eq!(g.check(&inside), Ok(()));

        let outside = BackendCmd::StartPodExec {
            namespace: s("default"),
            name: s("web-0"),
            container: None,
        };
        let err = g.check(&outside).unwrap_err();
        assert!(
            err.contains("StartPodExec") && err.contains("default"),
            "{err}"
        );

        // The rest of the exec commands stay read-only and always pass.
        for cmd in [
            BackendCmd::SendPodExecInput { data: s("bHM=") },
            BackendCmd::ResizePodExec { cols: 80, rows: 24 },
            BackendCmd::StopPodExec,
            BackendCmd::CheckPodExecAccess {
                namespace: s("default"),
                name: s("web-0"),
                container: None,
            },
        ] {
            assert_eq!(g.check(&cmd), Ok(()), "{cmd:?}");
        }
    }

    #[test]
    fn bulk_delete_needs_every_item_inside() {
        let g = guard();
        let all_inside = BackendCmd::BulkDelete {
            items: vec![(s("fathom-dev"), s("a")), (s("fathom-dev"), s("b"))],
        };
        assert_eq!(g.check(&all_inside), Ok(()));

        let one_outside = BackendCmd::BulkDelete {
            items: vec![(s("fathom-dev"), s("a")), (s("kube-system"), s("b"))],
        };
        let err = g.check(&one_outside).unwrap_err();
        assert!(err.contains("kube-system"), "{err}");

        assert_eq!(g.check(&BackendCmd::BulkDelete { items: vec![] }), Ok(()));
    }

    #[test]
    fn cluster_scoped_writes_are_refused() {
        let g = guard();
        for cmd in [
            BackendCmd::DeletePersistentVolume { name: s("pv") },
            BackendCmd::DeleteClusterRoleBinding { name: s("x") },
            BackendCmd::DeleteMutatingWebhookConfiguration { name: s("w") },
        ] {
            assert!(g.check(&cmd).is_err(), "{cmd:?}");
        }
    }

    #[test]
    fn context_switch_only_to_the_pinned_context() {
        let g = guard();
        assert_eq!(
            g.check(&BackendCmd::SetContext {
                context: s("kind-dev")
            }),
            Ok(())
        );
        let err = g
            .check(&BackendCmd::SetContext { context: s("prod") })
            .unwrap_err();
        assert!(err.contains("prod"), "{err}");
    }
}
