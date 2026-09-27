//! The write guard: which commands the dev bridge forwards to the backend.

use fathom::protocol::{BackendCmd, MutationTarget};

/// One object a strict-mode bridge may *create* (never replace, patch or delete).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTarget {
    pub kind: String,
    /// `None` for a cluster-scoped object.
    pub namespace: Option<String>,
    pub name: String,
}

impl CreateTarget {
    /// Parse `Kind/name` (cluster-scoped) or `Kind/namespace/name` (namespaced).
    pub fn parse(spec: &str) -> Result<Self, String> {
        let parts: Vec<&str> = spec.split('/').collect();
        if parts.iter().any(|p| p.trim().is_empty()) {
            return Err(format!(
                "'{spec}': empty segment (expected Kind/name or Kind/namespace/name)"
            ));
        }
        match parts.as_slice() {
            [kind, name] => Ok(Self {
                kind: (*kind).to_owned(),
                namespace: None,
                name: (*name).to_owned(),
            }),
            [kind, ns, name] => Ok(Self {
                kind: (*kind).to_owned(),
                namespace: Some((*ns).to_owned()),
                name: (*name).to_owned(),
            }),
            _ => Err(format!(
                "'{spec}': expected Kind/name or Kind/namespace/name"
            )),
        }
    }
}

impl std::fmt::Display for CreateTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{}/{ns}/{}", self.kind, self.name),
            None => write!(f, "{}/{}", self.kind, self.name),
        }
    }
}

/// Confines cluster writes to one context and one sandbox namespace.
///
/// Strict mode (`create_only: Some(..)`, set by `--read-only` / `--allow-create`) replaces the
/// namespace rule: every cluster write is refused except a `CreateResourceYaml` whose exact
/// kind/namespace/name is on the allow-list (an empty list is fully read-only). It lets a bridge
/// be pointed at a non-sandbox cluster for one narrowly scoped, create-only setup (see
/// `ui/tests/live/rbac-prod-scoped-setup.spec.ts`). `CreateResourceYaml` never replaces an
/// existing object, and the backend rejects YAML whose identity differs from the declared one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGuard {
    pub context: String,
    pub write_namespace: String,
    pub create_only: Option<Vec<CreateTarget>>,
}

impl WriteGuard {
    pub fn new(context: impl Into<String>, write_namespace: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            write_namespace: write_namespace.into(),
            create_only: None,
        }
    }

    /// Strict mode: only creates of exactly `targets` pass (empty = read-only).
    #[must_use]
    pub fn create_only(mut self, targets: Vec<CreateTarget>) -> Self {
        self.create_only = Some(targets);
        self
    }

    fn check_strict(cmd: &BackendCmd, targets: &[CreateTarget]) -> Result<(), String> {
        if let BackendCmd::CreateResourceYaml {
            kind,
            namespace,
            name,
            ..
        } = cmd
        {
            let wanted = CreateTarget {
                kind: kind.clone(),
                namespace: namespace.clone(),
                name: name.clone(),
            };
            if targets.contains(&wanted) {
                return Ok(());
            }
            return Err(format!(
                "CreateResourceYaml {wanted} is not on this bridge's --allow-create list"
            ));
        }
        Err(format!(
            "{} is a cluster write; this bridge is create-only/read-only",
            variant_name(cmd)
        ))
    }

    /// `Ok` if `cmd` may be forwarded, otherwise the reason it is refused.
    ///
    /// - `ReadOnly` and `Local` pass, with one exception: `SetMcpServerEnabled` is refused
    ///   outright. It classifies as `Local` (it writes a preference, not the cluster), but a
    ///   dev/debug bridge must never be able to expose an MCP endpoint. The backend also
    ///   hard-sets `Args::disable_mcp` in a devbridge shell, so this is defence in depth: the
    ///   command never even reaches it.
    /// - `Namespaced` passes only if every target namespace is the write namespace.
    /// - `ClusterScoped` never passes, except a `fathom-e2e-*` ClusterRole write (see
    ///   [`is_e2e_cluster_role_write`]).
    /// - `ContextSwitch` passes only to the guarded context.
    pub fn check(&self, cmd: &BackendCmd) -> Result<(), String> {
        if matches!(cmd, BackendCmd::SetMcpServerEnabled { .. }) {
            return Err(
                "SetMcpServerEnabled is refused; a dev bridge must never toggle the MCP server"
                    .to_owned(),
            );
        }
        // 037-node-shell: a privileged, host-namespace pod is never a sandbox operation, even
        // though it classifies as a namespaced write in the sandbox namespace.
        if matches!(cmd, BackendCmd::StartNodeShell { .. }) {
            return Err(
                "StartNodeShell is refused; a dev bridge must never create a privileged node pod"
                    .to_owned(),
            );
        }
        // 040-manifest-export: the kubeconfig export reads credentials of any context in the
        // kubeconfig, so a bridge pinned to one context only exports that one.
        if let BackendCmd::PreviewKubeconfigExport { context }
        | BackendCmd::ExportKubeconfig { context, .. } = cmd
            && *context != self.context
        {
            return Err(format!(
                "{} for context '{context}' refused; this bridge is pinned to context '{}'",
                variant_name(cmd),
                self.context
            ));
        }
        match cmd.mutation_target() {
            MutationTarget::ReadOnly | MutationTarget::Local => Ok(()),
            MutationTarget::ContextSwitch { context } if context == self.context => Ok(()),
            MutationTarget::ContextSwitch { context } => Err(format!(
                "SetContext to '{context}' refused; this bridge is pinned to context '{}'",
                self.context
            )),
            _ if self.create_only.is_some() => {
                Self::check_strict(cmd, self.create_only.as_deref().unwrap_or_default())
            }
            MutationTarget::ClusterScoped if is_e2e_cluster_role_write(cmd) => Ok(()),
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
        }
    }
}

/// Name prefix of the throwaway ClusterRoles the live suite may write (042-role-create).
pub const E2E_CLUSTER_ROLE_PREFIX: &str = "fathom-e2e-";

/// 042-role-create: the one cluster-scoped write a sandbox bridge forwards. A `ClusterRole`
/// named `fathom-e2e-*` may be created, replaced or deleted so the live suite can exercise the
/// ClusterRole create/edit flow. It grants nothing on its own: binding it cluster-wide needs a
/// `ClusterRoleBinding`, which stays refused, and the API server's escalation check still
/// applies to its rules. Never allowed in strict (`--read-only`/`--allow-create`) mode, which
/// is checked first.
fn is_e2e_cluster_role_write(cmd: &BackendCmd) -> bool {
    let name = match cmd {
        BackendCmd::CreateResourceYaml {
            kind,
            namespace: None,
            name,
            ..
        }
        | BackendCmd::ApplyResourceYaml {
            kind,
            namespace: None,
            name,
            ..
        } if kind == "ClusterRole" => name,
        BackendCmd::DeleteClusterRole { name } => name,
        _ => return false,
    };
    name.len() > E2E_CLUSTER_ROLE_PREFIX.len() && name.starts_with(E2E_CLUSTER_ROLE_PREFIX)
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

    /// 037-node-shell (T006): refused even when targeting the sandbox namespace; its cleanup
    /// and read-side companions still pass the normal classification.
    #[test]
    fn node_shell_start_is_refused_in_sandbox() {
        for namespace in ["fathom-dev", "default"] {
            let err = guard()
                .check(&BackendCmd::StartNodeShell {
                    node: s("n1"),
                    namespace: s(namespace),
                    image: s("busybox"),
                    image_pull_secret: None,
                    deadline_seconds: None,
                })
                .expect_err("StartNodeShell must be refused");
            assert!(err.contains("StartNodeShell"), "{err}");
        }
        assert!(
            guard()
                .check(&BackendCmd::CheckNodeShellAccess {
                    namespace: s("default")
                })
                .is_ok()
        );
        assert!(
            guard()
                .check(&BackendCmd::StopNodeShell {
                    session_id: 1,
                    namespace: s("default")
                })
                .is_err(),
            "stop targets a non-sandbox namespace, refused by the namespace rule"
        );
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

    /// 042-role-create: only `fathom-e2e-*` ClusterRoles pass, and never in strict mode.
    #[test]
    fn e2e_cluster_roles_are_the_only_cluster_scoped_write() {
        let g = guard();
        let apply = |kind: &str, name: &str| BackendCmd::ApplyResourceYaml {
            api_version: s("rbac.authorization.k8s.io/v1"),
            kind: s(kind),
            namespace: None,
            name: s(name),
            yaml: String::new(),
            expected_resource_version: s("1"),
        };
        for ok in [
            create("ClusterRole", None, "fathom-e2e-reader"),
            apply("ClusterRole", "fathom-e2e-reader"),
            BackendCmd::DeleteClusterRole {
                name: s("fathom-e2e-reader"),
            },
        ] {
            assert_eq!(g.check(&ok), Ok(()), "{ok:?}");
        }
        for refused in [
            create("ClusterRole", None, "cluster-admin"),
            create("ClusterRole", None, "fathom-e2e-"),
            apply("ClusterRole", "view"),
            create("ClusterRoleBinding", None, "fathom-e2e-reader"),
            apply("ClusterRoleBinding", "fathom-e2e-reader"),
            BackendCmd::DeleteClusterRole { name: s("admin") },
            BackendCmd::DeleteClusterRoleBinding {
                name: s("fathom-e2e-reader"),
            },
        ] {
            assert!(g.check(&refused).is_err(), "{refused:?}");
        }
        let strict = guard().create_only(vec![]);
        assert!(
            strict
                .check(&create("ClusterRole", None, "fathom-e2e-reader"))
                .is_err()
        );
    }

    fn create(kind: &str, namespace: Option<&str>, name: &str) -> BackendCmd {
        BackendCmd::CreateResourceYaml {
            api_version: s("rbac.authorization.k8s.io/v1"),
            kind: s(kind),
            namespace: namespace.map(s),
            name: s(name),
            yaml: String::new(),
        }
    }

    #[test]
    fn create_target_parses_both_scopes() {
        assert_eq!(
            CreateTarget::parse("ClusterRoleBinding/x").unwrap(),
            CreateTarget {
                kind: s("ClusterRoleBinding"),
                namespace: None,
                name: s("x")
            }
        );
        let t = CreateTarget::parse("RoleBinding/ns/x").unwrap();
        assert_eq!(t.namespace.as_deref(), Some("ns"));
        assert_eq!(t.to_string(), "RoleBinding/ns/x");
        for bad in ["", "x", "a//b", "a/b/c/d", "RoleBinding/ns/"] {
            assert!(CreateTarget::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn strict_mode_allows_only_listed_creates() {
        let g = guard().create_only(vec![
            CreateTarget::parse("ClusterRoleBinding/fathom-agent-view").unwrap(),
            CreateTarget::parse("RoleBinding/fathom-e2e/fathom-agent-admin").unwrap(),
        ]);
        assert_eq!(
            g.check(&create("ClusterRoleBinding", None, "fathom-agent-view")),
            Ok(())
        );
        assert_eq!(
            g.check(&create(
                "RoleBinding",
                Some("fathom-e2e"),
                "fathom-agent-admin"
            )),
            Ok(())
        );
        // Same name, other kind/scope/namespace: refused.
        for cmd in [
            create("ClusterRoleBinding", None, "cluster-admin"),
            create("RoleBinding", Some("default"), "fathom-agent-admin"),
            create("RoleBinding", Some("fathom-dev"), "fathom-agent-admin"),
            create("ClusterRole", None, "fathom-agent-view"),
        ] {
            assert!(g.check(&cmd).is_err(), "{cmd:?}");
        }
        // Every other write is refused, even inside the sandbox namespace.
        for cmd in [
            BackendCmd::ScaleDeployment {
                namespace: s("fathom-dev"),
                name: s("scale-me"),
                replicas: 2,
            },
            BackendCmd::DeleteClusterRoleBinding {
                name: s("fathom-agent-view"),
            },
            BackendCmd::ApplyResourceYaml {
                api_version: s("rbac.authorization.k8s.io/v1"),
                kind: s("ClusterRoleBinding"),
                namespace: None,
                name: s("fathom-agent-view"),
                yaml: String::new(),
                expected_resource_version: s("1"),
            },
        ] {
            assert!(g.check(&cmd).is_err(), "{cmd:?}");
        }
        // Reads and the pinned context still pass.
        assert_eq!(g.check(&BackendCmd::RefreshClusterRoles), Ok(()));
        assert_eq!(
            g.check(&BackendCmd::SetContext {
                context: s("kind-dev")
            }),
            Ok(())
        );
        assert!(
            g.check(&BackendCmd::SetContext { context: s("prod") })
                .is_err()
        );
    }

    #[test]
    fn kubeconfig_export_is_pinned_to_the_bridge_context() {
        let g = WriteGuard::new("kind-dev", "fathom-dev");
        assert_eq!(
            g.check(&BackendCmd::PreviewKubeconfigExport {
                context: s("kind-dev")
            }),
            Ok(())
        );
        assert!(
            g.check(&BackendCmd::PreviewKubeconfigExport { context: s("prod") })
                .is_err()
        );
        assert!(
            g.check(&BackendCmd::ExportKubeconfig {
                context: s("prod"),
                path: s("/tmp/k"),
                include_embedded_credentials: false,
                overwrite: false,
            })
            .is_err()
        );
    }

    #[test]
    fn read_only_mode_refuses_every_write() {
        let g = guard().create_only(vec![]);
        assert!(
            g.check(&create("RoleBinding", Some("fathom-dev"), "x"))
                .is_err()
        );
        assert!(
            g.check(&BackendCmd::DeletePod {
                namespace: s("fathom-dev"),
                name: s("p")
            })
            .is_err()
        );
        assert_eq!(g.check(&BackendCmd::RefreshPods), Ok(()));
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
