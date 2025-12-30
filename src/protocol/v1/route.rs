use std::{fmt, hash::Hash};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SectionKey(pub String);

impl SectionKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for SectionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    Global,
    Cluster,
    Namespaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Route {
    pub section: SectionKey,
    pub scope: ScopeKind,

    /// Optional backend-reported active context name.
    pub context: Option<String>,

    /// Optional namespace (for namespaced routes); may be None for “multi-namespace” views.
    pub namespace: Option<String>,
}

impl Route {
    pub fn global(section: SectionKey) -> Self {
        Self {
            section,
            scope: ScopeKind::Global,
            context: None,
            namespace: None,
        }
    }

    pub fn cluster(section: SectionKey, context: Option<String>) -> Self {
        Self {
            section,
            scope: ScopeKind::Cluster,
            context,
            namespace: None,
        }
    }

    pub fn namespaced(
        section: SectionKey,
        context: Option<String>,
        namespace: Option<String>,
    ) -> Self {
        Self {
            section,
            scope: ScopeKind::Namespaced,
            context,
            namespace,
        }
    }
}

pub mod section_keys {
    use super::SectionKey;

    pub fn system() -> SectionKey {
        SectionKey::new("system")
    }

    pub fn workloads_pods() -> SectionKey {
        SectionKey::new("workloads/pods")
    }

    pub fn overview() -> SectionKey {
        SectionKey::new("overview")
    }

    pub fn nodes() -> SectionKey {
        SectionKey::new("nodes")
    }

    pub fn workloads_deployments() -> SectionKey {
        SectionKey::new("workloads/deployments")
    }

    pub fn workloads_statefulsets() -> SectionKey {
        SectionKey::new("workloads/statefulsets")
    }

    pub fn workloads_daemonsets() -> SectionKey {
        SectionKey::new("workloads/daemonsets")
    }

    pub fn workloads_jobs() -> SectionKey {
        SectionKey::new("workloads/jobs")
    }

    pub fn workloads_cronjobs() -> SectionKey {
        SectionKey::new("workloads/cronjobs")
    }

    pub fn workloads_replicasets() -> SectionKey {
        SectionKey::new("workloads/replicasets")
    }

    pub fn workloads_replicationcontrollers() -> SectionKey {
        SectionKey::new("workloads/replicationcontrollers")
    }

    pub fn config_configmaps() -> SectionKey {
        SectionKey::new("config/configmaps")
    }

    pub fn config_secrets() -> SectionKey {
        SectionKey::new("config/secrets")
    }

    pub fn config_resourcequotas() -> SectionKey {
        SectionKey::new("config/resourcequotas")
    }

    pub fn config_limitranges() -> SectionKey {
        SectionKey::new("config/limitranges")
    }

    pub fn config_horizontalpodautoscalers() -> SectionKey {
        SectionKey::new("config/horizontalpodautoscalers")
    }

    pub fn config_verticalpodautoscalers() -> SectionKey {
        SectionKey::new("config/verticalpodautoscalers")
    }

    pub fn config_poddisruptionbudgets() -> SectionKey {
        SectionKey::new("config/poddisruptionbudgets")
    }

    pub fn config_priorityclasses() -> SectionKey {
        SectionKey::new("config/priorityclasses")
    }

    pub fn config_runtimeclasses() -> SectionKey {
        SectionKey::new("config/runtimeclasses")
    }

    pub fn config_leases() -> SectionKey {
        SectionKey::new("config/leases")
    }

    pub fn config_mutatingwebhooks() -> SectionKey {
        SectionKey::new("config/mutatingwebhooks")
    }

    pub fn config_validatingwebhooks() -> SectionKey {
        SectionKey::new("config/validatingwebhooks")
    }

    pub fn network_services() -> SectionKey {
        SectionKey::new("network/services")
    }

    pub fn network_ingress() -> SectionKey {
        SectionKey::new("network/ingress")
    }

    pub fn network_policies() -> SectionKey {
        SectionKey::new("network/networkpolicies")
    }

    pub fn storage_pv() -> SectionKey {
        SectionKey::new("storage/persistentvolumes")
    }

    pub fn storage_pvc() -> SectionKey {
        SectionKey::new("storage/persistentvolumeclaims")
    }

    pub fn storage_storageclasses() -> SectionKey {
        SectionKey::new("storage/storageclasses")
    }

    pub fn namespaces() -> SectionKey {
        SectionKey::new("namespaces")
    }

    pub fn events() -> SectionKey {
        SectionKey::new("events")
    }

    pub fn access_roles() -> SectionKey {
        SectionKey::new("access/roles")
    }

    pub fn access_rolebindings() -> SectionKey {
        SectionKey::new("access/rolebindings")
    }

    pub fn access_clusterroles() -> SectionKey {
        SectionKey::new("access/clusterroles")
    }

    pub fn access_clusterrolebindings() -> SectionKey {
        SectionKey::new("access/clusterrolebindings")
    }

    pub fn custom_resources() -> SectionKey {
        SectionKey::new("custom-resources")
    }

    pub fn integrations_argo() -> SectionKey {
        SectionKey::new("integrations/argo")
    }

    pub fn integrations_metrics() -> SectionKey {
        SectionKey::new("integrations/metrics")
    }

    pub fn integrations_grafana() -> SectionKey {
        SectionKey::new("integrations/grafana")
    }

    pub fn integrations_flux() -> SectionKey {
        SectionKey::new("integrations/flux")
    }

    pub fn network_portforwards() -> SectionKey {
        SectionKey::new("network/portforwards")
    }
}
