use crate::protocol::{ScopeKind, SectionKey, section_keys};

// Module declarations for different sections
pub mod access_control;
pub mod config;
pub mod custom_resources;
pub mod integrations;
pub mod network;
pub mod storage;
pub mod workloads;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavGroup {
    Overview,
    Nodes,
    Workloads,
    Config,
    Network,
    Storage,
    Namespaces,
    Events,
    AccessControl,
    CustomResources,
    Integrations,
}

#[derive(Debug, Clone)]
pub struct SectionDef {
    pub key: SectionKey,
    pub label: &'static str,
    pub group: NavGroup,
    pub scope: ScopeKind,
}

pub fn all_sections() -> Vec<SectionDef> {
    [
        overview_sections(),
        workload_sections(),
        config_sections(),
        network_sections(),
        storage_sections(),
        cluster_sections(),
        access_control_sections(),
        custom_resource_sections(),
        integration_sections(),
    ]
    .concat()
}

fn section(key: SectionKey, label: &'static str, group: NavGroup, scope: ScopeKind) -> SectionDef {
    SectionDef {
        key,
        label,
        group,
        scope,
    }
}

fn overview_sections() -> Vec<SectionDef> {
    let group = NavGroup::Overview;
    vec![
        section(
            section_keys::overview(),
            "Overview",
            group,
            ScopeKind::Cluster,
        ),
        section(section_keys::nodes(), "Nodes", group, ScopeKind::Cluster),
    ]
}

fn workload_sections() -> Vec<SectionDef> {
    let group = NavGroup::Workloads;
    vec![
        section(
            section_keys::workloads_pods(),
            "Pods",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::workloads_deployments(),
            "Deployments",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::workloads_statefulsets(),
            "StatefulSets",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::workloads_daemonsets(),
            "DaemonSets",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::workloads_jobs(),
            "Jobs",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::workloads_cronjobs(),
            "CronJobs",
            group,
            ScopeKind::Namespaced,
        ),
    ]
}

fn config_sections() -> Vec<SectionDef> {
    let group = NavGroup::Config;
    vec![
        section(
            section_keys::config_configmaps(),
            "ConfigMaps",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_secrets(),
            "Secrets",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_resourcequotas(),
            "Resource Quotas",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_limitranges(),
            "Limit Ranges",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_horizontalpodautoscalers(),
            "Horizontal Pod Autoscalers",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_verticalpodautoscalers(),
            "Vertical Pod Autoscalers",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_poddisruptionbudgets(),
            "Pod Disruption Budgets",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_priorityclasses(),
            "Priority Classes",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::config_runtimeclasses(),
            "Runtime Classes",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::config_leases(),
            "Leases",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::config_mutatingwebhooks(),
            "Mutating Webhook Configurations",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::config_validatingwebhooks(),
            "Validating Webhook Configurations",
            group,
            ScopeKind::Cluster,
        ),
    ]
}

fn network_sections() -> Vec<SectionDef> {
    let group = NavGroup::Network;
    vec![
        section(
            section_keys::network_services(),
            "Services",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::network_endpoints(),
            "Endpoints",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::network_ingress(),
            "Ingress",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::network_ingressclasses(),
            "IngressClasses",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::network_policies(),
            "NetworkPolicies",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::network_portforwards(),
            "Port Forwards",
            group,
            ScopeKind::Cluster,
        ),
    ]
}

fn storage_sections() -> Vec<SectionDef> {
    let group = NavGroup::Storage;
    vec![
        section(
            section_keys::storage_pv(),
            "PersistentVolumes",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::storage_pvc(),
            "PersistentVolumeClaims",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::storage_storageclasses(),
            "StorageClasses",
            group,
            ScopeKind::Cluster,
        ),
    ]
}

fn cluster_sections() -> Vec<SectionDef> {
    let group = NavGroup::Overview;
    vec![
        section(
            section_keys::namespaces(),
            "Namespaces",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::events(),
            "Events",
            group,
            ScopeKind::Namespaced,
        ),
    ]
}

fn access_control_sections() -> Vec<SectionDef> {
    let group = NavGroup::AccessControl;
    vec![
        section(
            section_keys::access_roles(),
            "Roles",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::access_rolebindings(),
            "RoleBindings",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::access_clusterroles(),
            "ClusterRoles",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::access_clusterrolebindings(),
            "ClusterRoleBindings",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::access_namespace_access(),
            "Namespace Access",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::access_oidc_config(),
            "OIDC Configuration",
            group,
            ScopeKind::Cluster,
        ),
    ]
}

fn custom_resource_sections() -> Vec<SectionDef> {
    let group = NavGroup::CustomResources;
    vec![section(
        section_keys::custom_resources(),
        "Custom Resources",
        group,
        ScopeKind::Cluster,
    )]
}

fn integration_sections() -> Vec<SectionDef> {
    let group = NavGroup::Integrations;
    vec![
        section(
            section_keys::integrations_argo(),
            "Argo",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::integrations_metrics(),
            "Metrics",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::integrations_grafana(),
            "Grafana",
            group,
            ScopeKind::Cluster,
        ),
        section(
            section_keys::integrations_flux(),
            "Flux",
            group,
            ScopeKind::Namespaced,
        ),
        section(
            section_keys::integrations_velero(),
            "Velero",
            group,
            ScopeKind::Namespaced,
        ),
    ]
}
pub fn find_section<'a>(sections: &'a [SectionDef], key: &SectionKey) -> Option<&'a SectionDef> {
    sections.iter().find(|s| &s.key == key)
}
