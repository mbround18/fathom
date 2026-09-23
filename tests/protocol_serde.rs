//! Serde round-trip tests for the v1 protocol (FR-026, contracts/ipc.md).
//!
//! Every `BackendCmd` and event variant is constructed with sample data and pushed through
//! `serde_json::to_value` → `from_value` → `to_value`. The `*_is_exhaustive` matches make
//! this file fail to compile when a variant is added without a sample here.
//!
//! The sample builders below were generated from the protocol definitions; keep them in
//! sync by hand (or regenerate) when payload structs change.

use fathom::protocol::*;
use fathom::sections::network;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn sample_route() -> Route {
    Route::namespaced(
        section_keys::workloads_pods(),
        Some("kind-a".to_owned()),
        Some("default".to_owned()),
    )
}

fn sample_ui_config() -> UiConfig {
    let mut ui = UiConfig::default();
    ui.theme.accent = ThemeAccent::Magenta;
    ui.density.override_density = Some(Density::Compact);
    ui.layouts.insert(
        WidthClass::Wide,
        LayoutProfile {
            slots: vec![
                SlotConfig {
                    kind: SlotKind::Nav,
                    size_pct: 15.0,
                    pinned_route: None,
                    tool: None,
                },
                SlotConfig {
                    kind: SlotKind::List,
                    size_pct: 55.5,
                    pinned_route: Some(sample_route()),
                    tool: None,
                },
                SlotConfig {
                    kind: SlotKind::Tool,
                    size_pct: 29.5,
                    pinned_route: None,
                    tool: Some(ToolKind::Logs),
                },
            ],
            dock: Some(DockConfig {
                open: true,
                height_pct: 30.0,
                tabs: vec![ToolKind::Events, ToolKind::Portforwards],
            }),
        },
    );
    ui.last_selection = Some(LastSelection {
        context: "kind-a".to_owned(),
        namespace: None,
        section: section_keys::workloads_pods(),
    });
    ui
}

fn sample_cluster_role_binding_details() -> ClusterRoleBindingDetails {
    ClusterRoleBindingDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        role_ref: "s".to_owned(),
        subjects: vec![sample_subject_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_cluster_role_binding_summary() -> ClusterRoleBindingSummary {
    ClusterRoleBindingSummary {
        name: "s".to_owned(),
        role_ref: "s".to_owned(),
        subjects_count: 7,
    }
}

fn sample_cluster_role_details() -> ClusterRoleDetails {
    ClusterRoleDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        rules: vec![sample_policy_rule_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_cluster_role_summary() -> ClusterRoleSummary {
    ClusterRoleSummary {
        name: "s".to_owned(),
        rules_count: 7,
    }
}

fn sample_config_map_details() -> ConfigMapDetails {
    ConfigMapDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        keys: vec!["s".to_owned()],
        data: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        raw_json: "s".to_owned(),
    }
}

fn sample_config_map_summary() -> ConfigMapSummary {
    ConfigMapSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        keys_count: 7,
    }
}

fn sample_cron_job_details() -> CronJobDetails {
    CronJobDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        schedule: "s".to_owned(),
        suspend: true,
        active: 7,
        last_schedule_time: Some("s".to_owned()),
        concurrency_policy: "s".to_owned(),
        starting_deadline_seconds: Some(7),
        successful_jobs_history_limit: Some(7),
        failed_jobs_history_limit: Some(7),
        labels: vec![("s".to_owned(), "s".to_owned())],
        annotations: vec![("s".to_owned(), "s".to_owned())],
    }
}

fn sample_cron_job_summary() -> CronJobSummary {
    CronJobSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        schedule: "s".to_owned(),
        suspend: true,
        active: 7,
        last_schedule: Some("s".to_owned()),
        age: "s".to_owned(),
    }
}

fn sample_disk_usage() -> DiskUsage {
    DiskUsage {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        provisioned_bytes: 7,
        used_bytes: Some(7),
        state: Some("s".to_owned()),
    }
}

fn sample_endpoint_details() -> EndpointDetails {
    EndpointDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        subsets: vec![sample_endpoint_subset()],
        raw_json: "s".to_owned(),
    }
}

fn sample_endpoint_summary() -> EndpointSummary {
    EndpointSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        ready_addresses_count: 7,
        not_ready_addresses_count: 7,
        port_count: 7,
    }
}

fn sample_events_summary() -> EventsSummary {
    EventsSummary {
        events: vec![sample_kubernetes_event()],
        total_count: 7,
    }
}

fn sample_flux_overview() -> FluxOverview {
    FluxOverview {
        installed: true,
        sources: vec![sample_flux_resource_summary()],
        kustomizations: vec![sample_flux_resource_summary()],
        helmreleases: vec![sample_flux_resource_summary()],
    }
}

fn sample_hubble_traffic_summary() -> HubbleTrafficSummary {
    HubbleTrafficSummary {
        installed: true,
        generated_at: "s".to_owned(),
        total_flows: 7,
        forwarded: 5,
        dropped: 2,
        top_pairs: vec![HubbleFlowPair {
            source_namespace: "s".to_owned(),
            dest_namespace: "s".to_owned(),
            count: 7,
        }],
        top_drop_reasons: vec![HubbleDropReason {
            reason: "POLICY_DENIED".to_owned(),
            count: 2,
        }],
    }
}

fn sample_horizontal_pod_autoscaler_details() -> HorizontalPodAutoscalerDetails {
    HorizontalPodAutoscalerDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        target_ref: "s".to_owned(),
        min_replicas: Some(7),
        max_replicas: 7,
        current_replicas: Some(7),
        desired_replicas: 7,
        conditions: vec!["s".to_owned()],
        raw_json: "s".to_owned(),
    }
}

fn sample_horizontal_pod_autoscaler_summary() -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        target_ref: "s".to_owned(),
        min_replicas: Some(7),
        max_replicas: 7,
        current_replicas: Some(7),
    }
}

fn sample_ingress_class_details() -> IngressClassDetails {
    IngressClassDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        controller: "s".to_owned(),
        parameters: Some(sample_ingress_class_parameters()),
        raw_json: "s".to_owned(),
    }
}

fn sample_ingress_class_summary() -> IngressClassSummary {
    IngressClassSummary {
        name: "s".to_owned(),
        controller: "s".to_owned(),
    }
}

fn sample_ingress_details() -> IngressDetails {
    IngressDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        ingress_class: Some("s".to_owned()),
        rules: vec![sample_ingress_rule()],
        default_backend: Some(sample_ingress_backend()),
        load_balancer_ingress: vec![sample_ingress_load_balancer_ingress()],
        tls_hosts: vec!["s".to_owned()],
        raw_json: "s".to_owned(),
    }
}

fn sample_ingress_summary() -> IngressSummary {
    IngressSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        ingress_class: Some("s".to_owned()),
        hosts_count: 7,
        load_balancer_ips: vec!["s".to_owned()],
    }
}

fn sample_job_details() -> JobDetails {
    JobDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        active: 7,
        succeeded: 7,
        failed: 7,
        completions: Some(7),
        parallelism: Some(7),
        start_time: Some("s".to_owned()),
        completion_time: Some("s".to_owned()),
        duration: "s".to_owned(),
        selector: "s".to_owned(),
        labels: vec![("s".to_owned(), "s".to_owned())],
        annotations: vec![("s".to_owned(), "s".to_owned())],
        conditions: vec![sample_job_condition()],
    }
}

fn sample_job_summary() -> JobSummary {
    JobSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        completions: "s".to_owned(),
        duration: "s".to_owned(),
        active: 7,
        succeeded: 7,
        failed: 7,
        age: "s".to_owned(),
    }
}

fn sample_lease_details() -> LeaseDetails {
    LeaseDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        holder_identity: Some("s".to_owned()),
        lease_duration_seconds: Some(7),
        acquire_time: Some("s".to_owned()),
        renew_time: Some("s".to_owned()),
        raw_json: "s".to_owned(),
    }
}

fn sample_lease_summary() -> LeaseSummary {
    LeaseSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        holder_identity: Some("s".to_owned()),
        renew_time: Some("s".to_owned()),
    }
}

fn sample_limit_range_details() -> LimitRangeDetails {
    LimitRangeDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        limits: vec![sample_limit_range_item_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_limit_range_summary() -> LimitRangeSummary {
    LimitRangeSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        limits_count: 7,
    }
}

fn sample_metrics_stack_scan_result() -> MetricsStackScanResult {
    MetricsStackScanResult {
        tier: MetricsStackTier::Full,
        components: vec![sample_detected_metrics_component()],
        suggested_endpoint: Some(sample_detected_metrics_component()),
    }
}

fn sample_mutating_webhook_configuration_details() -> MutatingWebhookConfigurationDetails {
    MutatingWebhookConfigurationDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        webhooks: vec![sample_mutating_webhook_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_mutating_webhook_configuration_summary() -> MutatingWebhookConfigurationSummary {
    MutatingWebhookConfigurationSummary {
        name: "s".to_owned(),
        webhooks_count: 7,
    }
}

fn sample_namespace_details() -> NamespaceDetails {
    NamespaceDetails {
        name: "s".to_owned(),
        status: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        creation_timestamp: "s".to_owned(),
        pod_count: 7,
        deployment_count: 7,
        statefulset_count: 7,
        daemonset_count: 7,
        job_count: 7,
        cronjob_count: 7,
        service_count: 7,
        configmap_count: 7,
        secret_count: 7,
        raw_json: "s".to_owned(),
    }
}

fn sample_namespace_summary() -> NamespaceSummary {
    NamespaceSummary {
        name: "s".to_owned(),
        status: "s".to_owned(),
        pod_count: 7,
        deployment_count: 7,
        creation_timestamp: "s".to_owned(),
    }
}

fn sample_network_policy_details() -> NetworkPolicyDetails {
    NetworkPolicyDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        pod_selector: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        policy_types: vec!["s".to_owned()],
        ingress_rules: vec![sample_policy_rule()],
        egress_rules: vec![sample_policy_rule()],
        raw_json: "s".to_owned(),
    }
}

fn sample_network_policy_summary() -> NetworkPolicySummary {
    NetworkPolicySummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        pod_selector: "s".to_owned(),
        ingress_rules_count: 7,
        egress_rules_count: 7,
    }
}

fn sample_node_details() -> NodeDetails {
    NodeDetails {
        name: "s".to_owned(),
        status: "s".to_owned(),
        roles: vec!["s".to_owned()],
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        creation_timestamp: "s".to_owned(),
        kernel_version: "s".to_owned(),
        container_runtime: "s".to_owned(),
        kubelet_version: "s".to_owned(),
        os: "s".to_owned(),
        arch: "s".to_owned(),
        capacity_cpu: "s".to_owned(),
        capacity_memory: "s".to_owned(),
        allocatable_cpu: "s".to_owned(),
        allocatable_memory: "s".to_owned(),
        pod_count: 7,
        pod_capacity: 7,
        conditions: vec![sample_node_condition()],
        raw_json: "s".to_owned(),
    }
}

fn sample_node_summary() -> NodeSummary {
    NodeSummary {
        name: "s".to_owned(),
        status: "s".to_owned(),
        role: "s".to_owned(),
        pod_count: 7,
        pod_capacity: 7,
        cpu_allocatable_millicores: 7,
        memory_allocatable_bytes: 7,
    }
}

fn sample_persistent_volume_claim_details() -> PersistentVolumeClaimDetails {
    PersistentVolumeClaimDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        status: "s".to_owned(),
        volume_name: Some("s".to_owned()),
        capacity: Some("s".to_owned()),
        access_modes: vec!["s".to_owned()],
        storage_class: Some("s".to_owned()),
        requested_storage: Some("s".to_owned()),
        raw_json: "s".to_owned(),
    }
}

fn sample_persistent_volume_claim_summary() -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        status: "s".to_owned(),
        volume_name: Some("s".to_owned()),
        capacity: Some("s".to_owned()),
        access_modes: vec!["s".to_owned()],
        storage_class: Some("s".to_owned()),
    }
}

fn sample_persistent_volume_details() -> PersistentVolumeDetails {
    PersistentVolumeDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        capacity: "s".to_owned(),
        access_modes: vec!["s".to_owned()],
        status: "s".to_owned(),
        storage_class: Some("s".to_owned()),
        reclaim_policy: "s".to_owned(),
        claim_ref: Some("s".to_owned()),
        volume_mode: Some("s".to_owned()),
        csi_driver: Some("s".to_owned()),
        raw_json: "s".to_owned(),
    }
}

fn sample_persistent_volume_summary() -> PersistentVolumeSummary {
    PersistentVolumeSummary {
        name: "s".to_owned(),
        capacity: "s".to_owned(),
        access_modes: vec!["s".to_owned()],
        status: "s".to_owned(),
        storage_class: Some("s".to_owned()),
        reclaim_policy: "s".to_owned(),
        claim_ref: Some("s".to_owned()),
    }
}

fn sample_pod_details() -> PodDetails {
    PodDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        containers: vec!["s".to_owned()],
        ports: vec![sample_container_port()],
        resource_specs: vec![sample_container_resource_spec()],
        owner: Some(sample_pod_owner()),
        resolved_owner: Some(sample_pod_owner()),
        persistent_volume_claims: vec!["s".to_owned()],
        raw_json: "s".to_owned(),
        containers_lifecycle: vec![sample_container_lifecycle()],
    }
}

fn sample_container_state() -> ContainerState {
    ContainerState {
        phase: "terminated".to_owned(),
        reason: Some("s".to_owned()),
        exit_code: Some(1),
        started_at: Some("s".to_owned()),
        finished_at: Some("s".to_owned()),
    }
}

fn sample_probe_spec() -> ProbeSpec {
    ProbeSpec {
        probe_type: "readiness".to_owned(),
        mechanism: "httpGet".to_owned(),
        target: "s".to_owned(),
        period_seconds: 10,
        failure_threshold: 3,
        success_threshold: 1,
    }
}

fn sample_lifecycle_hook_spec() -> LifecycleHookSpec {
    LifecycleHookSpec {
        hook_type: "preStop".to_owned(),
        mechanism: "exec".to_owned(),
        target: "s".to_owned(),
    }
}

fn sample_container_lifecycle() -> ContainerLifecycle {
    ContainerLifecycle {
        name: "s".to_owned(),
        is_init: false,
        image: "s".to_owned(),
        ready: true,
        state: sample_container_state(),
        restart_count: 1,
        last_state: Some(sample_container_state()),
        probes: vec![sample_probe_spec()],
        hooks: vec![sample_lifecycle_hook_spec()],
    }
}

fn sample_pod_disruption_budget_details() -> PodDisruptionBudgetDetails {
    PodDisruptionBudgetDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        min_available: Some("s".to_owned()),
        max_unavailable: Some("s".to_owned()),
        current_healthy: Some(7),
        desired_healthy: Some(7),
        disruptions_allowed: Some(7),
        expected_pods: Some(7),
        raw_json: "s".to_owned(),
    }
}

fn sample_pod_disruption_budget_summary() -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        min_available: Some("s".to_owned()),
        max_unavailable: Some("s".to_owned()),
        current_healthy: Some(7),
        desired_healthy: Some(7),
    }
}

fn sample_pod_metrics_history() -> PodMetricsHistory {
    PodMetricsHistory {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        containers: vec![sample_container_metrics_history()],
    }
}

fn sample_pod_metrics_summary() -> PodMetricsSummary {
    PodMetricsSummary {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        containers: vec![sample_container_metrics()],
    }
}

fn sample_pod_summary() -> PodSummary {
    PodSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        phase: "s".to_owned(),
        node: "s".to_owned(),
        ready: 7,
        total: 7,
        restarts: 7,
    }
}

fn sample_port_forward() -> network::PortForward {
    network::PortForward {
        id: "s".to_owned(),
        namespace: "s".to_owned(),
        pod: "s".to_owned(),
        local_port: 7,
        remote_port: 7,
        container: Some("s".to_owned()),
        status: network::PortForwardStatus::Active,
        auto_start: true,
    }
}

fn sample_priority_class_details() -> PriorityClassDetails {
    PriorityClassDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        value: 7,
        global_default: true,
        description: Some("s".to_owned()),
        preemption_policy: Some("s".to_owned()),
        raw_json: "s".to_owned(),
    }
}

fn sample_priority_class_summary() -> PriorityClassSummary {
    PriorityClassSummary {
        name: "s".to_owned(),
        value: 7,
        global_default: true,
        description: Some("s".to_owned()),
    }
}

fn sample_prometheus_metrics() -> PrometheusMetrics {
    PrometheusMetrics {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        cpu: sample_cpu_metrics(),
        memory: sample_memory_metrics(),
        network: sample_network_metrics(),
        timestamp: 7,
    }
}

fn sample_resource_quota_details() -> ResourceQuotaDetails {
    ResourceQuotaDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        hard: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        used: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        raw_json: "s".to_owned(),
    }
}

fn sample_resource_quota_summary() -> ResourceQuotaSummary {
    ResourceQuotaSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        hard_limits_count: 7,
    }
}

fn sample_role_binding_details() -> RoleBindingDetails {
    RoleBindingDetails {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        role_ref: "s".to_owned(),
        subjects: vec![sample_subject_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_role_binding_summary() -> RoleBindingSummary {
    RoleBindingSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        role_ref: "s".to_owned(),
        subjects_count: 7,
    }
}

fn sample_role_details() -> RoleDetails {
    RoleDetails {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        rules: vec![sample_policy_rule_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_role_summary() -> RoleSummary {
    RoleSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        rules_count: 7,
    }
}

fn sample_runtime_class_details() -> RuntimeClassDetails {
    RuntimeClassDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        handler: "s".to_owned(),
        scheduling: Some("s".to_owned()),
        raw_json: "s".to_owned(),
    }
}

fn sample_runtime_class_summary() -> RuntimeClassSummary {
    RuntimeClassSummary {
        name: "s".to_owned(),
        handler: "s".to_owned(),
    }
}

fn sample_secret_details() -> SecretDetails {
    SecretDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        secret_type: "s".to_owned(),
        keys: vec!["s".to_owned()],
        raw_json: "s".to_owned(),
    }
}

fn sample_secret_summary() -> SecretSummary {
    SecretSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        secret_type: "s".to_owned(),
        keys_count: 7,
    }
}

fn sample_service_details() -> ServiceDetails {
    ServiceDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        service_type: "s".to_owned(),
        cluster_ip: "s".to_owned(),
        cluster_ips: vec!["s".to_owned()],
        external_ips: vec!["s".to_owned()],
        external_name: Some("s".to_owned()),
        selector: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        ports: vec![sample_service_port()],
        session_affinity: "s".to_owned(),
        load_balancer_ip: Some("s".to_owned()),
        load_balancer_ingress_hosts: vec!["s".to_owned()],
        raw_json: "s".to_owned(),
    }
}

fn sample_service_summary() -> ServiceSummary {
    ServiceSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        service_type: "s".to_owned(),
        cluster_ip: "s".to_owned(),
        external_ips: vec!["s".to_owned()],
        port_count: 7,
    }
}

fn sample_storage_class_details() -> StorageClassDetails {
    StorageClassDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        provisioner: "s".to_owned(),
        reclaim_policy: Some("s".to_owned()),
        volume_binding_mode: Some("s".to_owned()),
        is_default: true,
        allow_volume_expansion: Some(true),
        parameters: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        raw_json: "s".to_owned(),
    }
}

fn sample_storage_class_summary() -> StorageClassSummary {
    StorageClassSummary {
        name: "s".to_owned(),
        provisioner: "s".to_owned(),
        reclaim_policy: Some("s".to_owned()),
        volume_binding_mode: Some("s".to_owned()),
        is_default: true,
    }
}

fn sample_crd_instance_details() -> CrdInstanceDetails {
    CrdInstanceDetails {
        group: "s".to_owned(),
        kind: "s".to_owned(),
        version: "s".to_owned(),
        namespace: Some("s".to_owned()),
        name: "s".to_owned(),
        resource_version: "s".to_owned(),
        raw_json: "s".to_owned(),
    }
}

fn sample_application_summary() -> ApplicationSummary {
    ApplicationSummary {
        instance: "s".to_owned(),
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        managed_by: ApplicationManagedBy::Helm,
        version: Some("1.2.3".to_owned()),
        creation_timestamp: "s".to_owned(),
        ready_count: 2,
        total_count: 3,
    }
}

fn sample_related_resource_ref() -> RelatedResourceRef {
    RelatedResourceRef {
        section: section_keys::workloads_pods(),
        kind: "Pod".to_owned(),
        namespace: Some("s".to_owned()),
        name: "s".to_owned(),
        relation: "owns".to_owned(),
        status_hint: Some("Running".to_owned()),
    }
}

fn sample_application_details() -> ApplicationDetails {
    ApplicationDetails {
        instance: "s".to_owned(),
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        managed_by: ApplicationManagedBy::Flux,
        version: Some("1.2.3".to_owned()),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        urls: vec!["http://s".to_owned()],
        related: vec![sample_related_resource_ref()],
    }
}

fn sample_validating_webhook_configuration_details() -> ValidatingWebhookConfigurationDetails {
    ValidatingWebhookConfigurationDetails {
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        webhooks: vec![sample_validating_webhook_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_validating_webhook_configuration_summary() -> ValidatingWebhookConfigurationSummary {
    ValidatingWebhookConfigurationSummary {
        name: "s".to_owned(),
        webhooks_count: 7,
    }
}

fn sample_workload_details() -> WorkloadDetails {
    WorkloadDetails {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        creation_timestamp: "s".to_owned(),
        labels: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        annotations: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        containers: vec!["s".to_owned()],
        ports: vec![sample_container_port()],
        pod_selector: "s".to_owned(),
        pods: vec![sample_pod_summary()],
        raw_json: "s".to_owned(),
    }
}

fn sample_workload_metrics_aggregate() -> WorkloadMetricsAggregate {
    WorkloadMetricsAggregate {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        workload_kind: "s".to_owned(),
        replica_count: 7,
        total_cpu_usage_percent: Some(1.5),
        total_memory_usage_bytes: Some(7),
        total_memory_limit_bytes: Some(7),
        total_memory_request_bytes: Some(7),
        total_cpu_request_millicores: Some(7),
        total_cpu_limit_millicores: Some(7),
        containers: vec![sample_container_metrics_aggregate()],
        timestamp: 7,
    }
}

fn sample_workload_summary() -> WorkloadSummary {
    WorkloadSummary {
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        desired: 7,
        ready: 7,
        updated: 7,
        available: 7,
    }
}

fn sample_container_metrics() -> ContainerMetrics {
    ContainerMetrics {
        name: "s".to_owned(),
        cpu: "s".to_owned(),
        memory: "s".to_owned(),
    }
}

fn sample_container_metrics_aggregate() -> ContainerMetricsAggregate {
    ContainerMetricsAggregate {
        name: "s".to_owned(),
        cpu_usage_percent: Some(1.5),
        memory_usage_bytes: Some(7),
        memory_limit_bytes: Some(7),
        memory_request_bytes: Some(7),
        cpu_request_millicores: Some(7),
        cpu_limit_millicores: Some(7),
    }
}

fn sample_container_metrics_history() -> ContainerMetricsHistory {
    ContainerMetricsHistory {
        name: "s".to_owned(),
        samples: vec![sample_metrics_sample()],
        cpu_request_millicores: Some(1.5),
        cpu_limit_millicores: Some(1.5),
        memory_request_bytes: Some(1.5),
        memory_limit_bytes: Some(1.5),
    }
}

fn sample_container_port() -> ContainerPort {
    ContainerPort {
        container: "s".to_owned(),
        name: Some("s".to_owned()),
        port: 7,
        protocol: "s".to_owned(),
    }
}

fn sample_container_resource_spec() -> ContainerResourceSpec {
    ContainerResourceSpec {
        container: "s".to_owned(),
        cpu_request_millicores: Some(1.5),
        cpu_limit_millicores: Some(1.5),
        memory_request_bytes: Some(1.5),
        memory_limit_bytes: Some(1.5),
    }
}

fn sample_cpu_metrics() -> CpuMetrics {
    CpuMetrics {
        usage_percent: Some(1.5),
        cores: Some(1.5),
    }
}

fn sample_detected_metrics_component() -> DetectedMetricsComponent {
    DetectedMetricsComponent {
        kind: MetricsComponentKind::Prometheus,
        namespace: "s".to_owned(),
        service: "s".to_owned(),
        port: 7,
    }
}

fn sample_endpoint_subset() -> EndpointSubset {
    EndpointSubset {
        ready_addresses: vec![sample_endpoint_address()],
        not_ready_addresses: vec![sample_endpoint_address()],
        ports: vec![sample_endpoint_port()],
    }
}

fn sample_helm_revision_summary() -> HelmRevisionSummary {
    HelmRevisionSummary {
        revision: 3,
        status: HelmReleaseStatus::Deployed,
        description: Some("Upgrade complete".to_owned()),
        updated: "s".to_owned(),
        app_version: Some("7.0".to_owned()),
        chart_version: "1.2.3".to_owned(),
    }
}

fn sample_gitops_origin_ref() -> GitOpsOriginRef {
    GitOpsOriginRef {
        controller: GitOpsController::Flux,
        section: section_keys::flux_resource(),
        kind: "HelmRelease".to_owned(),
        namespace: "flux-system".to_owned(),
        name: "redis".to_owned(),
        ready: FluxReadyState::Ready,
        message: Some("s".to_owned()),
    }
}

fn sample_helm_release_summary() -> HelmReleaseSummary {
    HelmReleaseSummary {
        name: "redis".to_owned(),
        namespace: "cache".to_owned(),
        chart: "redis".to_owned(),
        chart_version: "1.2.3".to_owned(),
        app_version: Some("7.0".to_owned()),
        revision: 3,
        status: HelmReleaseStatus::Deployed,
        updated: "s".to_owned(),
        degraded: false,
    }
}

fn sample_helm_release_details() -> HelmReleaseDetails {
    HelmReleaseDetails {
        name: "redis".to_owned(),
        namespace: "cache".to_owned(),
        chart: "redis".to_owned(),
        chart_version: "1.2.3".to_owned(),
        app_version: Some("7.0".to_owned()),
        revision: 3,
        status: HelmReleaseStatus::Deployed,
        updated: "s".to_owned(),
        description: Some("Upgrade complete".to_owned()),
        values_user_supplied: "replicas: 3\n".to_owned(),
        values_effective: "replicas: 3\n".to_owned(),
        manifest: "apiVersion: v1\nkind: Service\n".to_owned(),
        revisions: vec![sample_helm_revision_summary()],
        origin: Some(sample_gitops_origin_ref()),
        related: vec![sample_related_resource_ref()],
        degraded: false,
    }
}

fn sample_flux_resource_summary() -> FluxResourceSummary {
    FluxResourceSummary {
        kind: "s".to_owned(),
        name: "s".to_owned(),
        namespace: "s".to_owned(),
        ready: FluxReadyState::Ready,
        message: "s".to_owned(),
        revision: "s".to_owned(),
        source_ref: Some("s".to_owned()),
        suspended: true,
        interval: "s".to_owned(),
        last_transition: "s".to_owned(),
    }
}

fn sample_ingress_backend() -> IngressBackend {
    IngressBackend {
        service_name: Some("s".to_owned()),
        service_port: Some(7),
        resource_kind: Some("s".to_owned()),
        resource_name: Some("s".to_owned()),
    }
}

fn sample_ingress_class_parameters() -> IngressClassParameters {
    IngressClassParameters {
        api_group: Some("s".to_owned()),
        kind: "s".to_owned(),
        namespace: Some("s".to_owned()),
        name: "s".to_owned(),
    }
}

fn sample_ingress_load_balancer_ingress() -> IngressLoadBalancerIngress {
    IngressLoadBalancerIngress {
        hostname: Some("s".to_owned()),
        ip: Some("s".to_owned()),
        ports: vec![sample_ingress_load_balancer_port()],
    }
}

fn sample_ingress_rule() -> IngressRule {
    IngressRule {
        host: Some("s".to_owned()),
        paths: vec![sample_ingress_path()],
    }
}

fn sample_job_condition() -> JobCondition {
    JobCondition {
        condition_type: "s".to_owned(),
        status: "s".to_owned(),
        reason: Some("s".to_owned()),
        message: Some("s".to_owned()),
        last_transition: Some("s".to_owned()),
    }
}

fn sample_kubernetes_event() -> KubernetesEvent {
    KubernetesEvent {
        namespace: "s".to_owned(),
        name: "s".to_owned(),
        event_type: "s".to_owned(),
        involved_object_kind: "s".to_owned(),
        involved_object_name: "s".to_owned(),
        reason: "s".to_owned(),
        message: "s".to_owned(),
        count: 7,
        first_timestamp: "s".to_owned(),
        last_timestamp: "s".to_owned(),
        source: "s".to_owned(),
    }
}

fn sample_limit_range_item_summary() -> LimitRangeItemSummary {
    LimitRangeItemSummary {
        limit_type: "s".to_owned(),
        max: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        min: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        default: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
    }
}

fn sample_memory_metrics() -> MemoryMetrics {
    MemoryMetrics {
        usage_bytes: Some(7),
        usage_percent: Some(1.5),
        limit_bytes: Some(7),
    }
}

fn sample_mutating_webhook_summary() -> MutatingWebhookSummary {
    MutatingWebhookSummary {
        name: "s".to_owned(),
        failure_policy: Some("s".to_owned()),
        side_effects: Some("s".to_owned()),
    }
}

fn sample_network_metrics() -> NetworkMetrics {
    NetworkMetrics {
        bytes_in: Some(7),
        bytes_out: Some(7),
        packets_in: Some(7),
        packets_out: Some(7),
    }
}

fn sample_node_condition() -> NodeCondition {
    NodeCondition {
        condition_type: "s".to_owned(),
        status: "s".to_owned(),
        reason: "s".to_owned(),
        message: "s".to_owned(),
        last_transition_time: "s".to_owned(),
    }
}

fn sample_pod_owner() -> PodOwner {
    PodOwner {
        kind: "s".to_owned(),
        name: "s".to_owned(),
    }
}

fn sample_policy_rule() -> PolicyRule {
    PolicyRule {
        protocols: vec!["s".to_owned()],
        ports: vec![7],
        from_selectors: vec![sample_rule_selector()],
        to_selectors: vec![sample_rule_selector()],
    }
}

fn sample_policy_rule_summary() -> PolicyRuleSummary {
    PolicyRuleSummary {
        api_groups: vec!["s".to_owned()],
        resources: vec!["s".to_owned()],
        verbs: vec!["s".to_owned()],
    }
}

fn sample_service_port() -> ServicePort {
    ServicePort {
        name: Some("s".to_owned()),
        port: 7,
        target_port: Some("s".to_owned()),
        protocol: "s".to_owned(),
        node_port: Some(7),
    }
}

fn sample_subject_summary() -> SubjectSummary {
    SubjectSummary {
        kind: "s".to_owned(),
        name: "s".to_owned(),
        namespace: Some("s".to_owned()),
    }
}

fn sample_validating_webhook_summary() -> ValidatingWebhookSummary {
    ValidatingWebhookSummary {
        name: "s".to_owned(),
        failure_policy: Some("s".to_owned()),
        side_effects: Some("s".to_owned()),
    }
}

fn sample_endpoint_address() -> EndpointAddress {
    EndpointAddress {
        ip: "s".to_owned(),
        hostname: Some("s".to_owned()),
        node_name: Some("s".to_owned()),
        target_ref: Some("s".to_owned()),
    }
}

fn sample_endpoint_port() -> EndpointPort {
    EndpointPort {
        name: Some("s".to_owned()),
        port: 7,
        protocol: "s".to_owned(),
    }
}

fn sample_ingress_load_balancer_port() -> IngressLoadBalancerPort {
    IngressLoadBalancerPort {
        port: 7,
        protocol: Some("s".to_owned()),
    }
}

fn sample_ingress_path() -> IngressPath {
    IngressPath {
        path: Some("s".to_owned()),
        path_type: "s".to_owned(),
        backend: sample_ingress_backend(),
    }
}

fn sample_metrics_sample() -> MetricsSample {
    MetricsSample {
        timestamp: "s".to_owned(),
        cpu_millicores: 1.5,
        memory_bytes: 1.5,
    }
}

fn sample_rule_selector() -> RuleSelector {
    RuleSelector {
        selector_type: "s".to_owned(),
        labels: Some(BTreeMap::from([("k".to_owned(), "v".to_owned())])),
        ip_block: Some("s".to_owned()),
        except: vec!["s".to_owned()],
    }
}

/// Every `BackendCmd` variant. The match in `all_cmds_is_exhaustive` fails to compile when a variant is added.
#[allow(clippy::too_many_lines)] // one literal per protocol variant; splitting would hide gaps
fn all_cmds() -> Vec<BackendCmd> {
    vec![
        BackendCmd::Initialize,
        BackendCmd::Shutdown,
        BackendCmd::RefreshContexts,
        BackendCmd::SetContext {
            context: "s".to_owned(),
        },
        BackendCmd::RefreshNamespaces,
        BackendCmd::SetNamespaceFilter {
            namespaces: vec!["s".to_owned()],
        },
        BackendCmd::RefreshNodes,
        BackendCmd::SelectNode {
            name: "s".to_owned(),
        },
        BackendCmd::CordonNode {
            name: "s".to_owned(),
        },
        BackendCmd::UncordonNode {
            name: "s".to_owned(),
        },
        BackendCmd::DrainNode {
            name: "s".to_owned(),
            ignore_daemonsets: true,
            force_delete_empty_dir: false,
        },
        BackendCmd::CancelDrainNode {
            name: "s".to_owned(),
            request_id: "s".to_owned(),
        },
        BackendCmd::TaintNode {
            name: "s".to_owned(),
            op: fathom::protocol::TaintOp::Add,
            key: "s".to_owned(),
            value: Some("s".to_owned()),
            effect: "NoSchedule".to_owned(),
        },
        BackendCmd::RefreshEvents,
        BackendCmd::RefreshPods,
        BackendCmd::SelectPod {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeletePod {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::StartPodLogs {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            container: Some("s".to_owned()),
        },
        BackendCmd::StopPodLogs,
        BackendCmd::FetchPodMetrics {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::StartPodExec {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            container: Some("s".to_owned()),
        },
        BackendCmd::SendPodExecInput {
            data: "s".to_owned(),
        },
        BackendCmd::ResizePodExec { cols: 80, rows: 24 },
        BackendCmd::StopPodExec,
        BackendCmd::RefreshDeployments,
        BackendCmd::SelectDeployment {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::ScaleDeployment {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            replicas: 7,
        },
        BackendCmd::RestartDeployment {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteDeployment {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshDaemonSets,
        BackendCmd::SelectDaemonSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RestartDaemonSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteDaemonSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshStatefulSets,
        BackendCmd::SelectStatefulSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::ScaleStatefulSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            replicas: 7,
        },
        BackendCmd::RestartStatefulSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteStatefulSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshReplicaSets,
        BackendCmd::SelectReplicaSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::ScaleReplicaSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            replicas: 7,
        },
        BackendCmd::RestartReplicaSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteReplicaSet {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshReplicationControllers,
        BackendCmd::SelectReplicationController {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::ScaleReplicationController {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            replicas: 7,
        },
        BackendCmd::RestartReplicationController {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteReplicationController {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshJobs,
        BackendCmd::SelectJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshCronJobs,
        BackendCmd::SelectCronJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::SuspendCronJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            suspend: true,
        },
        BackendCmd::TriggerCronJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteCronJob {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::CheckPortAvailability { port: 7 },
        BackendCmd::StartPortForward {
            namespace: "s".to_owned(),
            pod: "s".to_owned(),
            local_port: 7,
            remote_port: 7,
            container: Some("s".to_owned()),
        },
        BackendCmd::StopPortForward {
            tunnel_id: "s".to_owned(),
        },
        BackendCmd::DeletePortForward {
            tunnel_id: "s".to_owned(),
        },
        BackendCmd::UpdatePortForward {
            tunnel_id: "s".to_owned(),
            local_port: 7,
            remote_port: 7,
            auto_start: true,
        },
        BackendCmd::RefreshPortForwards,
        BackendCmd::RefreshServices,
        BackendCmd::SelectService {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteService {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshEndpoints,
        BackendCmd::SelectEndpoint {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshIngresses,
        BackendCmd::SelectIngress {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteIngress {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshIngressClasses,
        BackendCmd::SelectIngressClass {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshNetworkPolicies,
        BackendCmd::SelectNetworkPolicy {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteNetworkPolicy {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshIntegrations,
        BackendCmd::StartMetricsPolling {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
            interval_secs: 7,
        },
        BackendCmd::StopMetricsPolling,
        BackendCmd::StartWorkloadMetricsPolling {
            namespace: "s".to_owned(),
            workload_name: "s".to_owned(),
            workload_kind: "s".to_owned(),
            pod_selector: "s".to_owned(),
            interval_secs: 7,
        },
        BackendCmd::StopWorkloadMetricsPolling,
        BackendCmd::ConfigurePrometheus {
            namespace: "s".to_owned(),
            service: "s".to_owned(),
            port: 7,
        },
        BackendCmd::TestPrometheusConnection {
            namespace: "s".to_owned(),
            service: "s".to_owned(),
            port: 7,
        },
        BackendCmd::ConfigureGrafana {
            base_url: "s".to_owned(),
            api_key: "s".to_owned(),
        },
        BackendCmd::TestGrafanaConnection {
            base_url: "s".to_owned(),
            api_key: "s".to_owned(),
        },
        BackendCmd::RefreshFlux,
        BackendCmd::RefreshHubble,
        BackendCmd::StartMcpServer,
        BackendCmd::StopMcpServer,
        BackendCmd::RefreshMcpStatus,
        BackendCmd::OpenMcpConfigDir,
        BackendCmd::SimulateSlowCall,
        BackendCmd::BulkDelete {
            items: vec![("s".to_owned(), "s".to_owned())],
        },
        BackendCmd::SetActiveSection {
            section: SectionKey::new("workloads/pods"),
        },
        BackendCmd::RefreshPersistentVolumes,
        BackendCmd::SelectPersistentVolume {
            name: "s".to_owned(),
        },
        BackendCmd::DeletePersistentVolume {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshPersistentVolumeClaims,
        BackendCmd::SelectPersistentVolumeClaim {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeletePersistentVolumeClaim {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshStorageClasses,
        BackendCmd::SelectStorageClass {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteStorageClass {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshRoles,
        BackendCmd::SelectRole {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteRole {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshRoleBindings,
        BackendCmd::SelectRoleBinding {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteRoleBinding {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshClusterRoles,
        BackendCmd::SelectClusterRole {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteClusterRole {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshClusterRoleBindings,
        BackendCmd::SelectClusterRoleBinding {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteClusterRoleBinding {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshConfigMaps,
        BackendCmd::SelectConfigMap {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteConfigMap {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshSecrets,
        BackendCmd::SelectSecret {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteSecret {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshResourceQuotas,
        BackendCmd::SelectResourceQuota {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteResourceQuota {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshLimitRanges,
        BackendCmd::SelectLimitRange {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteLimitRange {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshHorizontalPodAutoscalers,
        BackendCmd::SelectHorizontalPodAutoscaler {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteHorizontalPodAutoscaler {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshPodDisruptionBudgets,
        BackendCmd::SelectPodDisruptionBudget {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeletePodDisruptionBudget {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshPriorityClasses,
        BackendCmd::SelectPriorityClass {
            name: "s".to_owned(),
        },
        BackendCmd::DeletePriorityClass {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshRuntimeClasses,
        BackendCmd::SelectRuntimeClass {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteRuntimeClass {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshLeases,
        BackendCmd::SelectLease {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::DeleteLease {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshMutatingWebhookConfigurations,
        BackendCmd::SelectMutatingWebhookConfiguration {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteMutatingWebhookConfiguration {
            name: "s".to_owned(),
        },
        BackendCmd::RefreshValidatingWebhookConfigurations,
        BackendCmd::SelectValidatingWebhookConfiguration {
            name: "s".to_owned(),
        },
        BackendCmd::DeleteValidatingWebhookConfiguration {
            name: "s".to_owned(),
        },
        BackendCmd::LoadUiConfig,
        BackendCmd::SaveUiConfig {
            ui: sample_ui_config(),
            expanded_groups: vec!["s".to_owned()],
        },
        BackendCmd::SetMcpAutoStart { enabled: true },
        BackendCmd::ApplyResourceYaml {
            api_version: "apps/v1".to_owned(),
            kind: "Deployment".to_owned(),
            namespace: Some("s".to_owned()),
            name: "s".to_owned(),
            yaml: "spec: {}\n".to_owned(),
            expected_resource_version: "1".to_owned(),
        },
        BackendCmd::RefreshCrds,
        BackendCmd::SelectCrdKind {
            group: "s".to_owned(),
            kind: "s".to_owned(),
            version: "s".to_owned(),
        },
        BackendCmd::RefreshCrdInstances {
            group: "s".to_owned(),
            kind: "s".to_owned(),
            version: "s".to_owned(),
            namespace: Some("s".to_owned()),
        },
        BackendCmd::SelectCrdInstance {
            group: "s".to_owned(),
            kind: "s".to_owned(),
            version: "s".to_owned(),
            namespace: Some("s".to_owned()),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshApplications,
        BackendCmd::SelectApplication {
            namespace: "s".to_owned(),
            instance: "s".to_owned(),
        },
        BackendCmd::RefreshRelatedResources {
            section: section_keys::applications(),
            kind: "s".to_owned(),
            namespace: Some("s".to_owned()),
            name: "s".to_owned(),
        },
        BackendCmd::RefreshHelmReleases,
        BackendCmd::SelectHelmRelease {
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::SelectFluxResource {
            kind: "s".to_owned(),
            namespace: "s".to_owned(),
            name: "s".to_owned(),
        },
        BackendCmd::PreviewHelmRollback {
            namespace: "s".to_owned(),
            release: "s".to_owned(),
            target_revision: 1,
        },
        BackendCmd::ApplyHelmRollback {
            namespace: "s".to_owned(),
            release: "s".to_owned(),
            target_revision: 1,
            base_revision: 2,
        },
        BackendCmd::PreviewHelmUpgrade {
            namespace: "s".to_owned(),
            release: "s".to_owned(),
            values_yaml: "s".to_owned(),
            chart_version: Some("1.2.3".to_owned()),
        },
        BackendCmd::ApplyHelmUpgrade {
            namespace: "s".to_owned(),
            release: "s".to_owned(),
            values_yaml: "s".to_owned(),
            chart_version: Some("1.2.3".to_owned()),
            base_revision: 2,
        },
        BackendCmd::UninstallHelmRelease {
            namespace: "s".to_owned(),
            release: "s".to_owned(),
            base_revision: 2,
        },
    ]
}

#[allow(dead_code)]
#[allow(clippy::too_many_lines)] // a single exhaustive match is the point; it cannot be split
fn all_cmds_is_exhaustive(v: &BackendCmd) {
    match v {
        BackendCmd::Initialize => {}
        BackendCmd::Shutdown => {}
        BackendCmd::RefreshContexts => {}
        BackendCmd::SetContext { .. } => {}
        BackendCmd::RefreshNamespaces => {}
        BackendCmd::SetNamespaceFilter { .. } => {}
        BackendCmd::RefreshNodes => {}
        BackendCmd::SelectNode { .. } => {}
        BackendCmd::CordonNode { .. } => {}
        BackendCmd::UncordonNode { .. } => {}
        BackendCmd::DrainNode { .. } => {}
        BackendCmd::CancelDrainNode { .. } => {}
        BackendCmd::TaintNode { .. } => {}
        BackendCmd::RefreshEvents => {}
        BackendCmd::RefreshPods => {}
        BackendCmd::SelectPod { .. } => {}
        BackendCmd::DeletePod { .. } => {}
        BackendCmd::StartPodLogs { .. } => {}
        BackendCmd::StopPodLogs => {}
        BackendCmd::FetchPodMetrics { .. } => {}
        BackendCmd::StartPodExec { .. } => {}
        BackendCmd::SendPodExecInput { .. } => {}
        BackendCmd::ResizePodExec { .. } => {}
        BackendCmd::StopPodExec => {}
        BackendCmd::RefreshDeployments => {}
        BackendCmd::SelectDeployment { .. } => {}
        BackendCmd::ScaleDeployment { .. } => {}
        BackendCmd::RestartDeployment { .. } => {}
        BackendCmd::DeleteDeployment { .. } => {}
        BackendCmd::RefreshDaemonSets => {}
        BackendCmd::SelectDaemonSet { .. } => {}
        BackendCmd::RestartDaemonSet { .. } => {}
        BackendCmd::DeleteDaemonSet { .. } => {}
        BackendCmd::RefreshStatefulSets => {}
        BackendCmd::SelectStatefulSet { .. } => {}
        BackendCmd::ScaleStatefulSet { .. } => {}
        BackendCmd::RestartStatefulSet { .. } => {}
        BackendCmd::DeleteStatefulSet { .. } => {}
        BackendCmd::RefreshReplicaSets => {}
        BackendCmd::SelectReplicaSet { .. } => {}
        BackendCmd::ScaleReplicaSet { .. } => {}
        BackendCmd::RestartReplicaSet { .. } => {}
        BackendCmd::DeleteReplicaSet { .. } => {}
        BackendCmd::RefreshReplicationControllers => {}
        BackendCmd::SelectReplicationController { .. } => {}
        BackendCmd::ScaleReplicationController { .. } => {}
        BackendCmd::RestartReplicationController { .. } => {}
        BackendCmd::DeleteReplicationController { .. } => {}
        BackendCmd::RefreshJobs => {}
        BackendCmd::SelectJob { .. } => {}
        BackendCmd::DeleteJob { .. } => {}
        BackendCmd::RefreshCronJobs => {}
        BackendCmd::SelectCronJob { .. } => {}
        BackendCmd::SuspendCronJob { .. } => {}
        BackendCmd::TriggerCronJob { .. } => {}
        BackendCmd::DeleteCronJob { .. } => {}
        BackendCmd::CheckPortAvailability { .. } => {}
        BackendCmd::StartPortForward { .. } => {}
        BackendCmd::StopPortForward { .. } => {}
        BackendCmd::DeletePortForward { .. } => {}
        BackendCmd::UpdatePortForward { .. } => {}
        BackendCmd::RefreshPortForwards => {}
        BackendCmd::RefreshServices => {}
        BackendCmd::SelectService { .. } => {}
        BackendCmd::DeleteService { .. } => {}
        BackendCmd::RefreshEndpoints => {}
        BackendCmd::SelectEndpoint { .. } => {}
        BackendCmd::RefreshIngresses => {}
        BackendCmd::SelectIngress { .. } => {}
        BackendCmd::DeleteIngress { .. } => {}
        BackendCmd::RefreshIngressClasses => {}
        BackendCmd::SelectIngressClass { .. } => {}
        BackendCmd::RefreshNetworkPolicies => {}
        BackendCmd::SelectNetworkPolicy { .. } => {}
        BackendCmd::DeleteNetworkPolicy { .. } => {}
        BackendCmd::RefreshIntegrations => {}
        BackendCmd::StartMetricsPolling { .. } => {}
        BackendCmd::StopMetricsPolling => {}
        BackendCmd::StartWorkloadMetricsPolling { .. } => {}
        BackendCmd::StopWorkloadMetricsPolling => {}
        BackendCmd::ConfigurePrometheus { .. } => {}
        BackendCmd::TestPrometheusConnection { .. } => {}
        BackendCmd::ConfigureGrafana { .. } => {}
        BackendCmd::TestGrafanaConnection { .. } => {}
        BackendCmd::RefreshFlux => {}
        BackendCmd::RefreshHubble => {}
        BackendCmd::StartMcpServer => {}
        BackendCmd::StopMcpServer => {}
        BackendCmd::RefreshMcpStatus => {}
        BackendCmd::OpenMcpConfigDir => {}
        BackendCmd::SimulateSlowCall => {}
        BackendCmd::BulkDelete { .. } => {}
        BackendCmd::SetActiveSection { .. } => {}
        BackendCmd::RefreshPersistentVolumes => {}
        BackendCmd::SelectPersistentVolume { .. } => {}
        BackendCmd::DeletePersistentVolume { .. } => {}
        BackendCmd::RefreshPersistentVolumeClaims => {}
        BackendCmd::SelectPersistentVolumeClaim { .. } => {}
        BackendCmd::DeletePersistentVolumeClaim { .. } => {}
        BackendCmd::RefreshStorageClasses => {}
        BackendCmd::SelectStorageClass { .. } => {}
        BackendCmd::DeleteStorageClass { .. } => {}
        BackendCmd::RefreshRoles => {}
        BackendCmd::SelectRole { .. } => {}
        BackendCmd::DeleteRole { .. } => {}
        BackendCmd::RefreshRoleBindings => {}
        BackendCmd::SelectRoleBinding { .. } => {}
        BackendCmd::DeleteRoleBinding { .. } => {}
        BackendCmd::RefreshClusterRoles => {}
        BackendCmd::SelectClusterRole { .. } => {}
        BackendCmd::DeleteClusterRole { .. } => {}
        BackendCmd::RefreshClusterRoleBindings => {}
        BackendCmd::SelectClusterRoleBinding { .. } => {}
        BackendCmd::DeleteClusterRoleBinding { .. } => {}
        BackendCmd::RefreshConfigMaps => {}
        BackendCmd::SelectConfigMap { .. } => {}
        BackendCmd::DeleteConfigMap { .. } => {}
        BackendCmd::RefreshSecrets => {}
        BackendCmd::SelectSecret { .. } => {}
        BackendCmd::DeleteSecret { .. } => {}
        BackendCmd::RefreshResourceQuotas => {}
        BackendCmd::SelectResourceQuota { .. } => {}
        BackendCmd::DeleteResourceQuota { .. } => {}
        BackendCmd::RefreshLimitRanges => {}
        BackendCmd::SelectLimitRange { .. } => {}
        BackendCmd::DeleteLimitRange { .. } => {}
        BackendCmd::RefreshHorizontalPodAutoscalers => {}
        BackendCmd::SelectHorizontalPodAutoscaler { .. } => {}
        BackendCmd::DeleteHorizontalPodAutoscaler { .. } => {}
        BackendCmd::RefreshPodDisruptionBudgets => {}
        BackendCmd::SelectPodDisruptionBudget { .. } => {}
        BackendCmd::DeletePodDisruptionBudget { .. } => {}
        BackendCmd::RefreshPriorityClasses => {}
        BackendCmd::SelectPriorityClass { .. } => {}
        BackendCmd::DeletePriorityClass { .. } => {}
        BackendCmd::RefreshRuntimeClasses => {}
        BackendCmd::SelectRuntimeClass { .. } => {}
        BackendCmd::DeleteRuntimeClass { .. } => {}
        BackendCmd::RefreshLeases => {}
        BackendCmd::SelectLease { .. } => {}
        BackendCmd::DeleteLease { .. } => {}
        BackendCmd::RefreshMutatingWebhookConfigurations => {}
        BackendCmd::SelectMutatingWebhookConfiguration { .. } => {}
        BackendCmd::DeleteMutatingWebhookConfiguration { .. } => {}
        BackendCmd::RefreshValidatingWebhookConfigurations => {}
        BackendCmd::SelectValidatingWebhookConfiguration { .. } => {}
        BackendCmd::DeleteValidatingWebhookConfiguration { .. } => {}
        BackendCmd::LoadUiConfig => {}
        BackendCmd::SaveUiConfig { .. } => {}
        BackendCmd::SetMcpAutoStart { .. } => {}
        BackendCmd::ApplyResourceYaml { .. } => {}
        BackendCmd::RefreshCrds => {}
        BackendCmd::SelectCrdKind { .. } => {}
        BackendCmd::RefreshCrdInstances { .. } => {}
        BackendCmd::SelectCrdInstance { .. } => {}
        BackendCmd::RefreshApplications => {}
        BackendCmd::SelectApplication { .. } => {}
        BackendCmd::RefreshRelatedResources { .. } => {}
        BackendCmd::RefreshHelmReleases => {}
        BackendCmd::SelectHelmRelease { .. } => {}
        BackendCmd::SelectFluxResource { .. } => {}
        BackendCmd::PreviewHelmRollback { .. } => {}
        BackendCmd::ApplyHelmRollback { .. } => {}
        BackendCmd::PreviewHelmUpgrade { .. } => {}
        BackendCmd::ApplyHelmUpgrade { .. } => {}
        BackendCmd::UninstallHelmRelease { .. } => {}
    }
}

/// Every `SystemEvent` variant. The match in `all_system_events_is_exhaustive` fails to compile when a variant is added.
fn all_system_events() -> Vec<SystemEvent> {
    vec![
        SystemEvent::BackendStarted,
        SystemEvent::Notification {
            level: NotificationLevel::Info,
            message: "s".to_owned(),
        },
        SystemEvent::ContextsUpdated {
            contexts: vec!["s".to_owned()],
            current: Some("s".to_owned()),
        },
        SystemEvent::ContextChanged {
            current: Some("s".to_owned()),
        },
        SystemEvent::NamespaceFilterChanged {
            namespaces: vec!["s".to_owned()],
        },
        SystemEvent::PrometheusConnectionTest {
            success: true,
            message: "s".to_owned(),
        },
        SystemEvent::GrafanaConnectionTest {
            success: true,
            message: "s".to_owned(),
        },
        SystemEvent::MetricsStackScanned(sample_metrics_stack_scan_result()),
        SystemEvent::McpServerStatus {
            running: true,
            message: "s".to_owned(),
        },
        SystemEvent::PortAvailability {
            port: 7,
            available: true,
        },
        SystemEvent::DiskUsageUpdated(vec![sample_disk_usage()]),
        SystemEvent::FluxOverviewUpdated(sample_flux_overview()),
        SystemEvent::HubbleTrafficUpdated(sample_hubble_traffic_summary()),
        SystemEvent::HelmMutationPreviewReady(sample_helm_mutation_preview_ready()),
        SystemEvent::HelmMutationApplied(sample_helm_mutation_applied()),
        SystemEvent::HelmMutationFailed(sample_helm_mutation_failed()),
        SystemEvent::UiConfigLoaded {
            ui: Some(sample_ui_config()),
            expanded_groups: vec!["s".to_owned()],
            mcp_auto_start: true,
        },
        SystemEvent::CrdCatalogUpdated {
            entries: vec![sample_crd_catalog_entry()],
        },
        SystemEvent::CrdInstancesUpdated {
            group: "s".to_owned(),
            kind: "s".to_owned(),
            version: "s".to_owned(),
            namespace: Some("s".to_owned()),
            instances: vec!["{}".to_owned()],
        },
    ]
}

fn sample_helm_mutation_preview_ready() -> HelmMutationPreviewReady {
    HelmMutationPreviewReady {
        namespace: "s".to_owned(),
        release: "s".to_owned(),
        action: HelmMutationAction::Rollback,
        base_revision: 3,
        current_manifest: "apiVersion: v1\nkind: ConfigMap\n".to_owned(),
        target_manifest: "apiVersion: v1\nkind: ConfigMap\ndata: {}\n".to_owned(),
        target_revision: Some(2),
        target_chart_version: None,
        gitops_warning: Some(sample_gitops_origin_ref()),
    }
}

fn sample_helm_mutation_applied() -> HelmMutationApplied {
    HelmMutationApplied {
        namespace: "s".to_owned(),
        release: "s".to_owned(),
        action: HelmMutationAction::Upgrade,
        new_revision: Some(4),
    }
}

fn sample_helm_mutation_failed() -> HelmMutationFailed {
    HelmMutationFailed {
        namespace: "s".to_owned(),
        release: "s".to_owned(),
        action: HelmMutationAction::Uninstall,
        stage: HelmMutationStage::Apply,
        message: "s".to_owned(),
    }
}

fn sample_crd_catalog_entry() -> fathom::protocol::CrdCatalogEntry {
    fathom::protocol::CrdCatalogEntry {
        group: "source.toolkit.fluxcd.io".to_owned(),
        kind: "GitRepository".to_owned(),
        plural: "gitrepositories".to_owned(),
        versions: vec!["v1".to_owned()],
        scope: fathom::protocol::CrdScope::Namespaced,
        permission_denied: false,
    }
}

#[allow(dead_code)]
fn all_system_events_is_exhaustive(v: &SystemEvent) {
    match v {
        SystemEvent::BackendStarted => {}
        SystemEvent::Notification { .. } => {}
        SystemEvent::ContextsUpdated { .. } => {}
        SystemEvent::ContextChanged { .. } => {}
        SystemEvent::NamespaceFilterChanged { .. } => {}
        SystemEvent::PrometheusConnectionTest { .. } => {}
        SystemEvent::GrafanaConnectionTest { .. } => {}
        SystemEvent::MetricsStackScanned(..) => {}
        SystemEvent::McpServerStatus { .. } => {}
        SystemEvent::PortAvailability { .. } => {}
        SystemEvent::DiskUsageUpdated(..) => {}
        SystemEvent::FluxOverviewUpdated(..) => {}
        SystemEvent::HubbleTrafficUpdated(..) => {}
        SystemEvent::HelmMutationPreviewReady(..) => {}
        SystemEvent::HelmMutationApplied(..) => {}
        SystemEvent::HelmMutationFailed(..) => {}
        SystemEvent::UiConfigLoaded { .. } => {}
        SystemEvent::CrdCatalogUpdated { .. } => {}
        SystemEvent::CrdInstancesUpdated { .. } => {}
    }
}

/// Every `ResourceListEvent` variant. The match in `all_resource_list_events_is_exhaustive` fails to compile when a variant is added.
fn all_resource_list_events() -> Vec<ResourceListEvent> {
    vec![
        ResourceListEvent::NamespacesUpdated(vec!["s".to_owned()]),
        ResourceListEvent::NodesUpdated(vec![sample_node_summary()]),
        ResourceListEvent::NamespaceSummariesUpdated(vec![sample_namespace_summary()]),
        ResourceListEvent::PodsUpdated(vec![sample_pod_summary()]),
        ResourceListEvent::DeploymentsUpdated(vec![sample_workload_summary()]),
        ResourceListEvent::DaemonSetsUpdated(vec![sample_workload_summary()]),
        ResourceListEvent::StatefulSetsUpdated(vec![sample_workload_summary()]),
        ResourceListEvent::ReplicaSetsUpdated(vec![sample_workload_summary()]),
        ResourceListEvent::ReplicationControllersUpdated(vec![sample_workload_summary()]),
        ResourceListEvent::JobsUpdated(vec![sample_job_summary()]),
        ResourceListEvent::CronJobsUpdated(vec![sample_cron_job_summary()]),
        ResourceListEvent::EventsUpdated(sample_events_summary()),
        ResourceListEvent::PortForwardsUpdated(vec![sample_port_forward()]),
        ResourceListEvent::ServicesUpdated(vec![sample_service_summary()]),
        ResourceListEvent::EndpointsUpdated(vec![sample_endpoint_summary()]),
        ResourceListEvent::IngressesUpdated(vec![sample_ingress_summary()]),
        ResourceListEvent::IngressClassesUpdated(vec![sample_ingress_class_summary()]),
        ResourceListEvent::NetworkPoliciesUpdated(vec![sample_network_policy_summary()]),
        ResourceListEvent::PersistentVolumesUpdated(vec![sample_persistent_volume_summary()]),
        ResourceListEvent::PersistentVolumeClaimsUpdated(vec![
            sample_persistent_volume_claim_summary(),
        ]),
        ResourceListEvent::StorageClassesUpdated(vec![sample_storage_class_summary()]),
        ResourceListEvent::RolesUpdated(vec![sample_role_summary()]),
        ResourceListEvent::RoleBindingsUpdated(vec![sample_role_binding_summary()]),
        ResourceListEvent::ClusterRolesUpdated(vec![sample_cluster_role_summary()]),
        ResourceListEvent::ClusterRoleBindingsUpdated(vec![sample_cluster_role_binding_summary()]),
        ResourceListEvent::ConfigMapsUpdated(vec![sample_config_map_summary()]),
        ResourceListEvent::SecretsUpdated(vec![sample_secret_summary()]),
        ResourceListEvent::ResourceQuotasUpdated(vec![sample_resource_quota_summary()]),
        ResourceListEvent::LimitRangesUpdated(vec![sample_limit_range_summary()]),
        ResourceListEvent::HorizontalPodAutoscalersUpdated(vec![
            sample_horizontal_pod_autoscaler_summary(),
        ]),
        ResourceListEvent::PodDisruptionBudgetsUpdated(
            vec![sample_pod_disruption_budget_summary()],
        ),
        ResourceListEvent::PriorityClassesUpdated(vec![sample_priority_class_summary()]),
        ResourceListEvent::RuntimeClassesUpdated(vec![sample_runtime_class_summary()]),
        ResourceListEvent::LeasesUpdated(vec![sample_lease_summary()]),
        ResourceListEvent::MutatingWebhookConfigurationsUpdated(vec![
            sample_mutating_webhook_configuration_summary(),
        ]),
        ResourceListEvent::ValidatingWebhookConfigurationsUpdated(vec![
            sample_validating_webhook_configuration_summary(),
        ]),
        ResourceListEvent::ApplicationsUpdated(vec![sample_application_summary()]),
        ResourceListEvent::HelmReleasesUpdated(vec![sample_helm_release_summary()]),
    ]
}

#[allow(dead_code)]
fn all_resource_list_events_is_exhaustive(v: &ResourceListEvent) {
    match v {
        ResourceListEvent::NamespacesUpdated(..) => {}
        ResourceListEvent::NodesUpdated(..) => {}
        ResourceListEvent::NamespaceSummariesUpdated(..) => {}
        ResourceListEvent::PodsUpdated(..) => {}
        ResourceListEvent::DeploymentsUpdated(..) => {}
        ResourceListEvent::DaemonSetsUpdated(..) => {}
        ResourceListEvent::StatefulSetsUpdated(..) => {}
        ResourceListEvent::ReplicaSetsUpdated(..) => {}
        ResourceListEvent::ReplicationControllersUpdated(..) => {}
        ResourceListEvent::JobsUpdated(..) => {}
        ResourceListEvent::CronJobsUpdated(..) => {}
        ResourceListEvent::EventsUpdated(..) => {}
        ResourceListEvent::PortForwardsUpdated(..) => {}
        ResourceListEvent::ServicesUpdated(..) => {}
        ResourceListEvent::EndpointsUpdated(..) => {}
        ResourceListEvent::IngressesUpdated(..) => {}
        ResourceListEvent::IngressClassesUpdated(..) => {}
        ResourceListEvent::NetworkPoliciesUpdated(..) => {}
        ResourceListEvent::PersistentVolumesUpdated(..) => {}
        ResourceListEvent::PersistentVolumeClaimsUpdated(..) => {}
        ResourceListEvent::StorageClassesUpdated(..) => {}
        ResourceListEvent::RolesUpdated(..) => {}
        ResourceListEvent::RoleBindingsUpdated(..) => {}
        ResourceListEvent::ClusterRolesUpdated(..) => {}
        ResourceListEvent::ClusterRoleBindingsUpdated(..) => {}
        ResourceListEvent::ConfigMapsUpdated(..) => {}
        ResourceListEvent::SecretsUpdated(..) => {}
        ResourceListEvent::ResourceQuotasUpdated(..) => {}
        ResourceListEvent::LimitRangesUpdated(..) => {}
        ResourceListEvent::HorizontalPodAutoscalersUpdated(..) => {}
        ResourceListEvent::PodDisruptionBudgetsUpdated(..) => {}
        ResourceListEvent::PriorityClassesUpdated(..) => {}
        ResourceListEvent::RuntimeClassesUpdated(..) => {}
        ResourceListEvent::LeasesUpdated(..) => {}
        ResourceListEvent::MutatingWebhookConfigurationsUpdated(..) => {}
        ResourceListEvent::ValidatingWebhookConfigurationsUpdated(..) => {}
        ResourceListEvent::ApplicationsUpdated(..) => {}
        ResourceListEvent::HelmReleasesUpdated(..) => {}
    }
}

/// Every `ResourceReadEvent` variant. The match in `all_resource_read_events_is_exhaustive` fails to compile when a variant is added.
fn all_resource_read_events() -> Vec<ResourceReadEvent> {
    vec![
        ResourceReadEvent::NodeDetailsUpdated(sample_node_details()),
        ResourceReadEvent::NamespaceDetailsUpdated(sample_namespace_details()),
        ResourceReadEvent::PodDetailsUpdated(sample_pod_details()),
        ResourceReadEvent::PodMetricsUpdated(sample_pod_metrics_summary()),
        ResourceReadEvent::PodMetricsHistoryUpdated(sample_pod_metrics_history()),
        ResourceReadEvent::PrometheusMetricsUpdated(sample_prometheus_metrics()),
        ResourceReadEvent::DeploymentDetailsUpdated(sample_workload_details()),
        ResourceReadEvent::DeploymentMetricsUpdated(sample_workload_metrics_aggregate()),
        ResourceReadEvent::DaemonSetDetailsUpdated(sample_workload_details()),
        ResourceReadEvent::DaemonSetMetricsUpdated(sample_workload_metrics_aggregate()),
        ResourceReadEvent::StatefulSetDetailsUpdated(sample_workload_details()),
        ResourceReadEvent::StatefulSetMetricsUpdated(sample_workload_metrics_aggregate()),
        ResourceReadEvent::ReplicaSetDetailsUpdated(sample_workload_details()),
        ResourceReadEvent::ReplicationControllerDetailsUpdated(sample_workload_details()),
        ResourceReadEvent::JobDetailsUpdated(sample_job_details()),
        ResourceReadEvent::CronJobDetailsUpdated(sample_cron_job_details()),
        ResourceReadEvent::ServiceDetailsUpdated(sample_service_details()),
        ResourceReadEvent::EndpointDetailsUpdated(sample_endpoint_details()),
        ResourceReadEvent::IngressDetailsUpdated(sample_ingress_details()),
        ResourceReadEvent::IngressClassDetailsUpdated(sample_ingress_class_details()),
        ResourceReadEvent::NetworkPolicyDetailsUpdated(sample_network_policy_details()),
        ResourceReadEvent::PersistentVolumeDetailsUpdated(sample_persistent_volume_details()),
        ResourceReadEvent::PersistentVolumeClaimDetailsUpdated(
            sample_persistent_volume_claim_details(),
        ),
        ResourceReadEvent::StorageClassDetailsUpdated(sample_storage_class_details()),
        ResourceReadEvent::RoleDetailsUpdated(sample_role_details()),
        ResourceReadEvent::RoleBindingDetailsUpdated(sample_role_binding_details()),
        ResourceReadEvent::ClusterRoleDetailsUpdated(sample_cluster_role_details()),
        ResourceReadEvent::ClusterRoleBindingDetailsUpdated(sample_cluster_role_binding_details()),
        ResourceReadEvent::ConfigMapDetailsUpdated(sample_config_map_details()),
        ResourceReadEvent::SecretDetailsUpdated(sample_secret_details()),
        ResourceReadEvent::ResourceQuotaDetailsUpdated(sample_resource_quota_details()),
        ResourceReadEvent::LimitRangeDetailsUpdated(sample_limit_range_details()),
        ResourceReadEvent::HorizontalPodAutoscalerDetailsUpdated(
            sample_horizontal_pod_autoscaler_details(),
        ),
        ResourceReadEvent::PodDisruptionBudgetDetailsUpdated(sample_pod_disruption_budget_details()),
        ResourceReadEvent::PriorityClassDetailsUpdated(sample_priority_class_details()),
        ResourceReadEvent::RuntimeClassDetailsUpdated(sample_runtime_class_details()),
        ResourceReadEvent::LeaseDetailsUpdated(sample_lease_details()),
        ResourceReadEvent::MutatingWebhookConfigurationDetailsUpdated(
            sample_mutating_webhook_configuration_details(),
        ),
        ResourceReadEvent::ValidatingWebhookConfigurationDetailsUpdated(
            sample_validating_webhook_configuration_details(),
        ),
        ResourceReadEvent::CrdInstanceDetailsUpdated(sample_crd_instance_details()),
        ResourceReadEvent::ApplicationDetailsUpdated(sample_application_details()),
        ResourceReadEvent::ApplicationMetricsUpdated(sample_workload_metrics_aggregate()),
        ResourceReadEvent::RelatedResourcesUpdated {
            section: section_keys::applications(),
            kind: "Application".to_owned(),
            namespace: Some("s".to_owned()),
            name: "s".to_owned(),
            related: vec![sample_related_resource_ref()],
        },
        ResourceReadEvent::HelmReleaseDetailsUpdated(sample_helm_release_details()),
        ResourceReadEvent::FluxResourceDetailsUpdated(sample_flux_resource_summary()),
        ResourceReadEvent::FluxResourceNotFound {
            kind: "HelmRelease".to_owned(),
            namespace: "flux-system".to_owned(),
            name: "redis".to_owned(),
        },
    ]
}

#[allow(dead_code)]
fn all_resource_read_events_is_exhaustive(v: &ResourceReadEvent) {
    match v {
        ResourceReadEvent::NodeDetailsUpdated(..) => {}
        ResourceReadEvent::NamespaceDetailsUpdated(..) => {}
        ResourceReadEvent::PodDetailsUpdated(..) => {}
        ResourceReadEvent::PodMetricsUpdated(..) => {}
        ResourceReadEvent::PodMetricsHistoryUpdated(..) => {}
        ResourceReadEvent::PrometheusMetricsUpdated(..) => {}
        ResourceReadEvent::DeploymentDetailsUpdated(..) => {}
        ResourceReadEvent::DeploymentMetricsUpdated(..) => {}
        ResourceReadEvent::DaemonSetDetailsUpdated(..) => {}
        ResourceReadEvent::DaemonSetMetricsUpdated(..) => {}
        ResourceReadEvent::StatefulSetDetailsUpdated(..) => {}
        ResourceReadEvent::StatefulSetMetricsUpdated(..) => {}
        ResourceReadEvent::ReplicaSetDetailsUpdated(..) => {}
        ResourceReadEvent::ReplicationControllerDetailsUpdated(..) => {}
        ResourceReadEvent::JobDetailsUpdated(..) => {}
        ResourceReadEvent::CronJobDetailsUpdated(..) => {}
        ResourceReadEvent::ServiceDetailsUpdated(..) => {}
        ResourceReadEvent::EndpointDetailsUpdated(..) => {}
        ResourceReadEvent::IngressDetailsUpdated(..) => {}
        ResourceReadEvent::IngressClassDetailsUpdated(..) => {}
        ResourceReadEvent::NetworkPolicyDetailsUpdated(..) => {}
        ResourceReadEvent::PersistentVolumeDetailsUpdated(..) => {}
        ResourceReadEvent::PersistentVolumeClaimDetailsUpdated(..) => {}
        ResourceReadEvent::StorageClassDetailsUpdated(..) => {}
        ResourceReadEvent::RoleDetailsUpdated(..) => {}
        ResourceReadEvent::RoleBindingDetailsUpdated(..) => {}
        ResourceReadEvent::ClusterRoleDetailsUpdated(..) => {}
        ResourceReadEvent::ClusterRoleBindingDetailsUpdated(..) => {}
        ResourceReadEvent::ConfigMapDetailsUpdated(..) => {}
        ResourceReadEvent::SecretDetailsUpdated(..) => {}
        ResourceReadEvent::ResourceQuotaDetailsUpdated(..) => {}
        ResourceReadEvent::LimitRangeDetailsUpdated(..) => {}
        ResourceReadEvent::HorizontalPodAutoscalerDetailsUpdated(..) => {}
        ResourceReadEvent::PodDisruptionBudgetDetailsUpdated(..) => {}
        ResourceReadEvent::PriorityClassDetailsUpdated(..) => {}
        ResourceReadEvent::RuntimeClassDetailsUpdated(..) => {}
        ResourceReadEvent::LeaseDetailsUpdated(..) => {}
        ResourceReadEvent::MutatingWebhookConfigurationDetailsUpdated(..) => {}
        ResourceReadEvent::ValidatingWebhookConfigurationDetailsUpdated(..) => {}
        ResourceReadEvent::CrdInstanceDetailsUpdated(..) => {}
        ResourceReadEvent::ApplicationDetailsUpdated(..) => {}
        ResourceReadEvent::ApplicationMetricsUpdated(..) => {}
        ResourceReadEvent::RelatedResourcesUpdated { .. } => {}
        ResourceReadEvent::HelmReleaseDetailsUpdated(..) => {}
        ResourceReadEvent::FluxResourceDetailsUpdated(..) => {}
        ResourceReadEvent::FluxResourceNotFound { .. } => {}
    }
}

/// Every `StreamEvent` variant. The match in `all_stream_events_is_exhaustive` fails to compile when a variant is added.
fn all_stream_events() -> Vec<StreamEvent> {
    vec![
        StreamEvent::PodLogsChunk {
            stream_id: StreamId(3),
            lines: vec!["s".to_owned()],
        },
        StreamEvent::PodExecReady {
            stream_id: StreamId(3),
        },
        StreamEvent::PodExecOutputChunk {
            stream_id: StreamId(3),
            data: "s".to_owned(),
        },
        StreamEvent::PortForwardTunnel {
            tunnel_id: "s".to_owned(),
            status: PortForwardStatus::Connecting,
            local_endpoint: Some("s".to_owned()),
        },
        StreamEvent::StreamClosed {
            stream_id: StreamId(3),
            reason: "s".to_owned(),
        },
        StreamEvent::ResourceApplied {
            request_id: "s".to_owned(),
            resource_version: "s".to_owned(),
        },
        StreamEvent::DrainProgress {
            request_id: "s".to_owned(),
            total_pods: 3,
            evicted: vec!["s".to_owned()],
            blocked_on: vec![("s".to_owned(), "s".to_owned())],
            ignored_daemonset_pods: vec!["s".to_owned()],
        },
        StreamEvent::NodeActionFinished {
            request_id: "s".to_owned(),
            ok: true,
            error: None,
        },
    ]
}

#[allow(dead_code)]
fn all_stream_events_is_exhaustive(v: &StreamEvent) {
    match v {
        StreamEvent::PodLogsChunk { .. } => {}
        StreamEvent::PodExecReady { .. } => {}
        StreamEvent::PodExecOutputChunk { .. } => {}
        StreamEvent::PortForwardTunnel { .. } => {}
        StreamEvent::StreamClosed { .. } => {}
        StreamEvent::ResourceApplied { .. } => {}
        StreamEvent::DrainProgress { .. } => {}
        StreamEvent::NodeActionFinished { .. } => {}
    }
}

fn all_events() -> Vec<BackendEvent> {
    let mut events = Vec::new();
    events.extend(all_system_events().into_iter().map(BackendEvent::System));
    events.extend(
        all_resource_list_events()
            .into_iter()
            .map(BackendEvent::ResourceList),
    );
    events.extend(
        all_resource_read_events()
            .into_iter()
            .map(BackendEvent::ResourceRead),
    );
    events.extend(all_stream_events().into_iter().map(BackendEvent::Stream));
    events.push(BackendEvent::CommandFinished {
        ok: true,
        error: None,
    });
    events.push(BackendEvent::CommandFinished {
        ok: false,
        error: Some("Failed to delete 2 item(s):\na/x: boom\nb/y: boom".to_owned()),
    });
    events
}

/// `to_value` → `from_value` → `to_value` must be stable. Returns the first JSON value.
fn assert_round_trip<T: Serialize + DeserializeOwned>(item: &T) -> Value {
    let first = serde_json::to_value(item).expect("serialize");
    let back: T = serde_json::from_value(first.clone())
        .unwrap_or_else(|e| panic!("deserialize {first}: {e}"));
    let second = serde_json::to_value(&back).expect("re-serialize");
    assert_eq!(first, second, "unstable round trip");
    first
}

/// Adjacently tagged: `{ "type": <variant> }` for unit variants, plus `"data"` otherwise.
fn assert_adjacently_tagged(value: &Value) {
    let obj = value.as_object().expect("enum serializes as an object");
    assert!(
        obj.get("type").is_some_and(Value::is_string),
        "missing string `type` tag: {value}"
    );
    assert!(
        obj.keys().all(|k| k == "type" || k == "data"),
        "unexpected keys: {value}"
    );
}

#[test]
fn every_command_round_trips() {
    let cmds = all_cmds();
    assert!(
        cmds.len() >= 140,
        "expected every variant, got {}",
        cmds.len()
    );
    for cmd in &cmds {
        let value = assert_round_trip(cmd);
        assert_adjacently_tagged(&value);
    }
}

#[test]
fn every_event_round_trips_in_an_envelope() {
    let events = all_events();
    assert!(
        events.len() >= 90,
        "expected every variant, got {}",
        events.len()
    );
    for (seq, event) in events.into_iter().enumerate() {
        let envelope = BackendEventEnvelope::new(
            seq as u64,
            sample_route(),
            Correlation::Request(RequestId(9)),
            event,
        );
        let value = assert_round_trip(&envelope);
        assert_eq!(value["version"], json!(1));
        assert_adjacently_tagged(&value["event"]);
        // Category events nest a second tagged enum; `CommandFinished` carries a plain struct.
        if value["event"]["type"] != "CommandFinished" {
            assert_adjacently_tagged(&value["event"]["data"]);
        }
        assert_adjacently_tagged(&value["correlation"]);
    }
}

#[test]
fn snapshot_unit_commands() {
    assert_eq!(
        serde_json::to_value(BackendCmd::RefreshPods).unwrap(),
        json!({ "type": "RefreshPods" })
    );
    assert_eq!(
        serde_json::to_value(BackendCmd::Shutdown).unwrap(),
        json!({ "type": "Shutdown" })
    );
}

#[test]
fn snapshot_struct_command() {
    let cmd = BackendCmd::SetContext {
        context: "kind-a".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&cmd).unwrap(),
        json!({ "type": "SetContext", "data": { "context": "kind-a" } })
    );

    let cmd = BackendCmd::BulkDelete {
        items: vec![("default".to_owned(), "web-0".to_owned())],
    };
    assert_eq!(
        serde_json::to_value(&cmd).unwrap(),
        json!({ "type": "BulkDelete", "data": { "items": [["default", "web-0"]] } })
    );
}

#[test]
fn snapshot_resource_list_envelope() {
    let envelope = BackendEventEnvelope::new(
        42,
        sample_route(),
        Correlation::Request(RequestId(7)),
        BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(vec![PodSummary {
            name: "web-0".to_owned(),
            namespace: "default".to_owned(),
            phase: "Running".to_owned(),
            node: "node-1".to_owned(),
            ready: 1,
            total: 1,
            restarts: 0,
        }])),
    );
    assert_eq!(
        serde_json::to_value(&envelope).unwrap(),
        json!({
            "version": 1,
            "seq": 42,
            "route": {
                "section": "workloads/pods",
                "scope": "Namespaced",
                "context": "kind-a",
                "namespace": "default"
            },
            "correlation": { "type": "Request", "data": 7 },
            "event": {
                "type": "ResourceList",
                "data": {
                    "type": "PodsUpdated",
                    "data": [{
                        "name": "web-0",
                        "namespace": "default",
                        "phase": "Running",
                        "node": "node-1",
                        "ready": 1,
                        "total": 1,
                        "restarts": 0
                    }]
                }
            }
        })
    );
}

#[test]
fn snapshot_system_event_and_correlation_none() {
    let envelope = BackendEventEnvelope::new(
        1,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    );
    assert_eq!(
        serde_json::to_value(&envelope).unwrap(),
        json!({
            "version": 1,
            "seq": 1,
            "route": { "section": "system", "scope": "Global", "context": null, "namespace": null },
            "correlation": { "type": "None" },
            "event": { "type": "System", "data": { "type": "BackendStarted" } }
        })
    );
}

#[test]
fn default_ui_config_shape() {
    let value = assert_round_trip(&UiConfig::default());
    assert_eq!(
        value,
        json!({
            "schema": 1,
            "theme": { "mode": "dark", "accent": "cyan", "effects": "on", "glitch": false },
            "density": {},
            "layouts": {}
        })
    );
}

#[test]
fn ui_config_camel_case_and_optional_fields() {
    let value = assert_round_trip(&sample_ui_config());
    assert_eq!(value["density"], json!({ "override": "compact" }));
    let wide = &value["layouts"]["wide"];
    assert_eq!(wide["slots"][1]["sizePct"], json!(55.5));
    assert_eq!(
        wide["slots"][1]["pinnedRoute"]["section"],
        json!("workloads/pods")
    );
    assert!(wide["slots"][0].get("pinnedRoute").is_none());
    assert_eq!(wide["dock"]["heightPct"], json!(30.0));
    assert_eq!(value["lastSelection"]["context"], json!("kind-a"));
}

#[test]
fn ui_config_rejects_wrong_schema_and_bad_shapes() {
    let mut v = serde_json::to_value(UiConfig::default()).unwrap();
    v["schema"] = json!(2);
    assert!(UiConfig::from_value(v).is_err());

    assert!(
        UiConfig::from_value(json!({ "theme": {} })).is_err(),
        "schema is required"
    );
    assert!(UiConfig::from_value(json!({ "schema": 1, "theme": { "mode": "neon" } })).is_err());

    let mut v = serde_json::to_value(sample_ui_config()).unwrap();
    v["layouts"]["wide"]["slots"][0]["sizePct"] = json!(250.0);
    assert!(UiConfig::from_value(v).is_err());

    // Missing optional sections fall back to defaults.
    assert_eq!(
        UiConfig::from_value(json!({ "schema": 1 })).unwrap(),
        UiConfig::default()
    );
}

#[test]
fn event_batch_and_bridge_types() {
    let batch = EventBatch {
        envelopes: vec![BackendEventEnvelope::new(
            1,
            Route::global(section_keys::system()),
            Correlation::Stream(StreamId(4)),
            BackendEvent::Stream(StreamEvent::StreamClosed {
                stream_id: StreamId(4),
                reason: "eof".to_owned(),
            }),
        )],
        dropped: 3,
    };
    let value = assert_round_trip(&batch);
    assert_eq!(value["dropped"], json!(3));
    assert_eq!(
        value["envelopes"][0]["correlation"],
        json!({ "type": "Stream", "data": 4 })
    );

    let info = SubscriptionInfo {
        protocol_version: BRIDGE_PROTOCOL_VERSION,
        backend_started: true,
    };
    assert_eq!(
        assert_round_trip(&info),
        json!({ "protocolVersion": 1, "backendStarted": true })
    );
    assert_eq!(
        assert_round_trip(&BridgeError::BackendGone),
        json!({ "type": "BackendGone" })
    );
    assert_eq!(
        assert_round_trip(&BridgeError::Forbidden("outside fathom-dev".to_owned())),
        json!({ "type": "Forbidden", "data": "outside fathom-dev" })
    );
}

#[test]
fn snapshot_command_finished() {
    let ok = BackendEventEnvelope::new(
        7,
        Route::global(section_keys::system()),
        Correlation::Request(RequestId(3)),
        BackendEvent::CommandFinished {
            ok: true,
            error: None,
        },
    );
    let value = assert_round_trip(&ok);
    assert_eq!(
        value["event"],
        json!({ "type": "CommandFinished", "data": { "ok": true, "error": null } })
    );
    assert_eq!(
        value["correlation"],
        json!({ "type": "Request", "data": 3 })
    );

    let failed = BackendEvent::CommandFinished {
        ok: false,
        error: Some("Not connected yet.".to_owned()),
    };
    assert_eq!(
        assert_round_trip(&failed),
        json!({
            "type": "CommandFinished",
            "data": { "ok": false, "error": "Not connected yet." }
        })
    );
}

#[test]
fn dev_bridge_messages() {
    let send = DevBridgeClientMsg::Send {
        cmd: BackendCmd::RefreshPods,
    };
    assert_eq!(
        assert_round_trip(&send),
        json!({ "op": "send", "cmd": { "type": "RefreshPods" } })
    );
    assert_eq!(
        assert_round_trip(&DevBridgeClientMsg::Subscribe),
        json!({ "op": "subscribe" })
    );

    assert_eq!(
        assert_round_trip(&DevBridgeServerMsg::Sent {
            request_id: RequestId(12)
        }),
        json!({ "op": "sent", "request_id": 12 })
    );
    assert_eq!(
        assert_round_trip(&DevBridgeServerMsg::Error {
            error: BridgeError::Forbidden("cluster-scoped".to_owned())
        }),
        json!({ "op": "error", "error": { "type": "Forbidden", "data": "cluster-scoped" } })
    );
    assert_eq!(
        assert_round_trip(&DevBridgeServerMsg::Subscribed {
            info: SubscriptionInfo {
                protocol_version: BRIDGE_PROTOCOL_VERSION,
                backend_started: false,
            }
        }),
        json!({ "op": "subscribed", "info": { "protocolVersion": 1, "backendStarted": false } })
    );
    let batch = assert_round_trip(&DevBridgeServerMsg::Batch {
        batch: EventBatch {
            envelopes: vec![],
            dropped: 2,
        },
    });
    assert_eq!(
        batch,
        json!({ "op": "batch", "batch": { "envelopes": [], "dropped": 2 } })
    );
}

/// `seq`, `RequestId`, and `StreamId` are exported to TypeScript as `number`, which is
/// exact only up to 2^53 - 1 (`Number.MAX_SAFE_INTEGER`). They are counters starting at 0,
/// so this holds in practice; pin that they serialize as plain JSON numbers.
#[test]
fn counters_serialize_as_plain_numbers() {
    const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
    assert_eq!(
        serde_json::to_value(RequestId(MAX_SAFE_INTEGER)).unwrap(),
        json!(9_007_199_254_740_991_u64)
    );
    assert_eq!(serde_json::to_value(StreamId(0)).unwrap(), json!(0));
    let seq_max = serde_json::to_value(BackendEventEnvelope::new(
        MAX_SAFE_INTEGER,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    ))
    .unwrap()["seq"]
        .clone();
    assert!(seq_max.is_u64());
    assert!(seq_max.as_f64().unwrap() as u64 == MAX_SAFE_INTEGER);
}
