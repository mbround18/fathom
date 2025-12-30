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
    vec![
        SectionDef {
            key: section_keys::overview(),
            label: "Overview",
            group: NavGroup::Overview,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::nodes(),
            label: "Nodes",
            group: NavGroup::Nodes,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::workloads_pods(),
            label: "Pods",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::workloads_deployments(),
            label: "Deployments",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::workloads_statefulsets(),
            label: "StatefulSets",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::workloads_daemonsets(),
            label: "DaemonSets",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::workloads_jobs(),
            label: "Jobs",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::workloads_cronjobs(),
            label: "CronJobs",
            group: NavGroup::Workloads,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_configmaps(),
            label: "ConfigMaps",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_secrets(),
            label: "Secrets",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_resourcequotas(),
            label: "Resource Quotas",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_limitranges(),
            label: "Limit Ranges",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_horizontalpodautoscalers(),
            label: "Horizontal Pod Autoscalers",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_verticalpodautoscalers(),
            label: "Vertical Pod Autoscalers",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_poddisruptionbudgets(),
            label: "Pod Disruption Budgets",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_priorityclasses(),
            label: "Priority Classes",
            group: NavGroup::Config,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::config_runtimeclasses(),
            label: "Runtime Classes",
            group: NavGroup::Config,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::config_leases(),
            label: "Leases",
            group: NavGroup::Config,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::config_mutatingwebhooks(),
            label: "Mutating Webhook Configurations",
            group: NavGroup::Config,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::config_validatingwebhooks(),
            label: "Validating Webhook Configurations",
            group: NavGroup::Config,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::network_services(),
            label: "Services",
            group: NavGroup::Network,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::network_ingress(),
            label: "Ingress",
            group: NavGroup::Network,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::network_policies(),
            label: "NetworkPolicies",
            group: NavGroup::Network,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::network_portforwards(),
            label: "Port Forwards",
            group: NavGroup::Network,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::storage_pv(),
            label: "PersistentVolumes",
            group: NavGroup::Storage,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::storage_pvc(),
            label: "PersistentVolumeClaims",
            group: NavGroup::Storage,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::storage_storageclasses(),
            label: "StorageClasses",
            group: NavGroup::Storage,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::namespaces(),
            label: "Namespaces",
            group: NavGroup::Namespaces,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::events(),
            label: "Events",
            group: NavGroup::Events,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::access_roles(),
            label: "Roles",
            group: NavGroup::AccessControl,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::access_rolebindings(),
            label: "RoleBindings",
            group: NavGroup::AccessControl,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::access_clusterroles(),
            label: "ClusterRoles",
            group: NavGroup::AccessControl,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::access_clusterrolebindings(),
            label: "ClusterRoleBindings",
            group: NavGroup::AccessControl,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::custom_resources(),
            label: "Custom Resources",
            group: NavGroup::CustomResources,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::integrations_argo(),
            label: "Argo",
            group: NavGroup::Integrations,
            scope: ScopeKind::Namespaced,
        },
        SectionDef {
            key: section_keys::integrations_metrics(),
            label: "Metrics",
            group: NavGroup::Integrations,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::integrations_grafana(),
            label: "Grafana",
            group: NavGroup::Integrations,
            scope: ScopeKind::Cluster,
        },
        SectionDef {
            key: section_keys::integrations_flux(),
            label: "Flux",
            group: NavGroup::Integrations,
            scope: ScopeKind::Namespaced,
        },
    ]
}

pub fn find_section<'a>(sections: &'a [SectionDef], key: &SectionKey) -> Option<&'a SectionDef> {
    sections.iter().find(|s| &s.key == key)
}
