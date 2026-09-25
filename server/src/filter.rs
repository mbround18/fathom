//! Hosted-mode command filter (spec 031, T017): the security boundary between a browser
//! session and the per-user backend.
//!
//! Cluster writes are NOT filtered here: the API server enforces the signed-in user's own
//! RBAC. This filter removes everything that would touch the server host instead of the
//! cluster (kind, kubeconfig, contexts, MCP, fleet, agent credentials) or the state directory
//! shared by all users of the process (`config.json`, integrations, team store, cluster
//! settings). Port-forward registry persistence is skipped inside the backend for hosted
//! access (see `save_port_forwards_to_disk`), so port-forward commands are allowed.

use fathom::protocol::{BackendCmdV1, HostedDenial, HostedPolicy};

/// A command refused in hosted mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub command: String,
    pub reason: HostedDenial,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "command {} is not available in hosted mode ({:?})",
            self.command, self.reason
        )
    }
}

impl std::error::Error for Denied {}

/// The serde variant name of a command (the `type` tag).
pub fn variant_name(cmd: &BackendCmdV1) -> String {
    serde_json::to_value(cmd)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str().map(str::to_owned)))
        .unwrap_or_else(|| "?".to_owned())
}

/// Allow or refuse `cmd` for a session whose backend is on `home` context.
pub fn check(cmd: &BackendCmdV1, home: &str) -> Result<(), Denied> {
    match cmd.hosted_policy(home) {
        HostedPolicy::Allowed => Ok(()),
        HostedPolicy::Denied(reason) => Err(Denied {
            command: variant_name(cmd),
            reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    const HOME: &str = "in-cluster";
    const CMD_SRC: &str = include_str!("../../src/protocol/v1/cmd.rs");

    fn parse(v: serde_json::Value) -> Option<BackendCmdV1> {
        serde_json::from_value(v).ok()
    }

    /// Variant names as declared in the enum source.
    fn declared_variants() -> Vec<String> {
        let start = CMD_SRC.find("pub enum BackendCmdV1").unwrap();
        let body = &CMD_SRC[start..];
        let mut depth = 0i32;
        let mut names = Vec::new();
        let mut first = true;
        for line in body.lines() {
            let t = line.trim();
            if depth == 1 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let n: String = t.chars().take_while(|c| c.is_alphanumeric()).collect();
                names.push(n);
            }
            for c in line.chars() {
                match c {
                    '{' | '(' => depth += 1,
                    '}' | ')' => depth -= 1,
                    _ => {}
                }
            }
            if !first && depth == 0 {
                break;
            }
            first = false;
        }
        names
    }

    /// Every variant, with the hosted verdict it must have. `true` = allowed.
    fn table() -> BTreeMap<&'static str, bool> {
        const DENIED: &[&str] = &[
            "Shutdown",
            "RefreshContexts",
            "SetContext",
            "RefreshKindPrerequisites",
            "RefreshKindClusters",
            "CreateKindCluster",
            "CancelCreateKindCluster",
            "DeleteKindCluster",
            "LoadMcpServerStatus",
            "SetMcpServerEnabled",
            "StartMcpDemoChat",
            "StopMcpDemoChat",
            "SendMcpDemoChatInput",
            "ResizeMcpDemoChat",
            "StartFleetPolling",
            "StopFleetPolling",
            "LoadUiConfig",
            "SaveUiConfig",
            "LoadClusterSettings",
            "SaveClusterSettings",
            "RefreshTeamStoreStatus",
            "SetTeamStorePath",
            "RefreshConnectionProfiles",
            "PublishConnectionProfile",
            "AdoptConnectionProfile",
            "RemoveConnectionProfile",
            "PublishOrgClusterSettings",
            "AcceptOrgClusterSettings",
            "DismissOrgClusterSettings",
            "ConfigurePrometheus",
            "ConfigureGrafana",
            "TestPrometheusConnection",
            "TestGrafanaConnection",
            "SimulateSlowCall",
            "GetNodeShellSettings",
            "SetNodeShellSettings",
            "SetExecPolicy",
            "LoadOidcConfig",
            "OidcSaveIssuer",
            "OidcDeleteIssuer",
            "OidcSetRoleMap",
            "OidcSignIn",
            "OidcCancelSignIn",
            "OidcSignOut",
            "SetContextAuthMode",
            "IdpSetAdminKey",
            "IdpRefreshDirectory",
            "LinkTeamProfile",
            "UnlinkTeamProfile",
            "PreviewRoleAssignment",
            "RefreshOrphanedSubjects",
            "SetDaemonPreference",
            "RefreshDaemon",
            "RequestDaemonTimeline",
            "RequestDaemonTopology",
            "RequestDaemonMetrics",
            "AiProviderLoginStart",
            "AiProviderSubmitApiKey",
            "AiProviderLoginCancel",
            "AiProviderSignOut",
            "AiSetActiveProvider",
            "AiSetOfflineMode",
            "AiOllamaProbe",
            "AiOllamaSelectModel",
            "AiSetOllamaEndpoint",
            "AiEnableFileStore",
            "AiGetStatus",
            "FederateContext",
            "UnfederateContext",
            "RequestFederationSnapshot",
        ];
        let mut t = BTreeMap::new();
        for n in declared_variants() {
            let n: &'static str = Box::leak(n.into_boxed_str());
            let denied = DENIED.contains(&n) || n.starts_with("Agent");
            t.insert(n, !denied);
        }
        t
    }

    #[test]
    fn table_covers_every_declared_variant() {
        let names = declared_variants();
        assert!(names.len() > 200, "variant scan broke: {}", names.len());
        assert!(names.contains(&"StartPodExec".to_owned()));
        assert!(names.contains(&"StartFleetPolling".to_owned()));
    }

    #[test]
    fn every_unit_variant_matches_the_table() {
        let t = table();
        let mut checked = 0;
        for (name, allowed) in &t {
            if *name == "SetContext" {
                continue;
            }
            if let Some(cmd) = parse(json!({ "type": name })) {
                assert_eq!(
                    check(&cmd, HOME).is_ok(),
                    *allowed,
                    "unit variant {name} misclassified"
                );
                checked += 1;
            }
        }
        assert!(checked > 40, "only {checked} unit variants exercised");
    }

    #[test]
    fn denied_struct_variants_are_refused_with_a_reason() {
        let s = |k: &str| json!(k);
        let samples = vec![
            ("CancelCreateKindCluster", json!({"request_id": "r"})),
            ("DeleteKindCluster", json!({"request_id": "r", "name": "n"})),
            ("SetMcpServerEnabled", json!({"enabled": true})),
            ("SendMcpDemoChatInput", json!({"data": "x"})),
            ("ResizeMcpDemoChat", json!({"cols": 1, "rows": 1})),
            ("LoadClusterSettings", json!({"context": s("c")})),
            ("SetTeamStorePath", json!({"path": "/tmp/x"})),
            ("AdoptConnectionProfile", json!({"profile_id": "p"})),
            ("RemoveConnectionProfile", json!({"profile_id": "p"})),
            ("PublishOrgClusterSettings", json!({"context": "c"})),
            ("AcceptOrgClusterSettings", json!({"context": "c"})),
            ("DismissOrgClusterSettings", json!({"context": "c"})),
            (
                "ConfigurePrometheus",
                json!({"namespace":"n","service":"s","port":9090}),
            ),
            (
                "ConfigureGrafana",
                json!({"base_url":"http://x","api_key":"k"}),
            ),
            (
                "TestPrometheusConnection",
                json!({"namespace":"n","service":"s","port":9090}),
            ),
            (
                "TestGrafanaConnection",
                json!({"base_url":"http://x","api_key":"k"}),
            ),
            (
                "AgentOpenConversation",
                json!({"object":{"kind":"Pod","namespace":"n","name":"p"}}),
            ),
            (
                "AgentAsk",
                json!({"object":{"kind":"Pod","namespace":"n","name":"p"},"question":"q"}),
            ),
            (
                "AgentReviewSuggestedAction",
                json!({"object":{"kind":"Pod","namespace":"n","name":"p"},"suggestion_id":"s"}),
            ),
        ];
        for (name, data) in samples {
            let cmd = parse(json!({"type": name, "data": data}))
                .unwrap_or_else(|| panic!("sample for {name} failed to deserialize"));
            let err = check(&cmd, HOME).expect_err(name);
            assert_eq!(err.command, name);
        }
    }

    #[test]
    fn set_context_only_to_home() {
        let home = parse(json!({"type":"SetContext","data":{"context":HOME}})).unwrap();
        assert!(check(&home, HOME).is_ok());
        for other in ["kind-dev", "", "In-Cluster", "in-cluster "] {
            let c = parse(json!({"type":"SetContext","data":{"context":other}})).unwrap();
            assert_eq!(
                check(&c, HOME).unwrap_err().reason,
                HostedDenial::ContextSwitch
            );
        }
    }

    #[test]
    fn cluster_operations_are_allowed_for_the_api_server_to_judge() {
        let ok = [
            json!({"type":"DeletePod","data":{"namespace":"n","name":"p"}}),
            json!({"type":"RefreshPods"}),
            json!({"type":"Initialize"}),
            json!({"type":"StopPodExec"}),
            json!({"type":"RefreshPortForwards"}),
        ];
        for v in ok {
            if let Some(cmd) = parse(v.clone()) {
                assert!(check(&cmd, HOME).is_ok(), "{v}");
            }
        }
    }

    #[test]
    fn denial_message_names_the_command() {
        let cmd = parse(json!({"type":"RefreshContexts"})).unwrap();
        let e = check(&cmd, HOME).unwrap_err();
        assert_eq!(e.reason, HostedDenial::Contexts);
        assert!(e.to_string().contains("RefreshContexts"));
    }
}
