# Parity Sign-off Checklist (T124, SC-001)

**Purpose**: prove every egui capability in FR-001-FR-003 and every `src/app/views/*.rs` file has a new-UI equivalent with an E2E test, before T125 deletes egui.

**Rule**: a row is ticked `[x]` only when a Playwright spec under `ui/tests/e2e/` exercises the new-UI equivalent (evidence = that spec file, found by grepping the section key or label). Rows with only Vitest coverage are left `[ ]` and listed under Gaps. Paths below are relative to `ui/` unless they start with `src/app`.

Evidence method: `src/resources/nav.ts` section keys were grepped against `tests/e2e/*.spec.ts` (key, or the quoted nav label). Unit coverage: `tests/unit/descriptors.test.ts` runs the column-budget/wiring contract over every registered descriptor (list fixture, row key, refresh, read, detail sections), so every kind below has at least that.

## FR-001 resource kinds (list and detail)

| Done | egui view | New-UI equivalent | E2E spec(s) |
|---|---|---|---|
| [x] | `pods.rs` | `src/resources/pods.tsx` (`workloads/pods`) | `us1-pods-list`, `us1-inspector`, `us1-pod-logs`, `perf-10k` |
| [x] | `workloads.rs` (deployments) | `src/resources/deployments.ts` | `us3-scale`, `us3-bulk-delete`, `us1-namespace-filter` |
| [ ] | `workloads.rs` (statefulsets) | `src/resources/statefulsets.ts` | none (unit only) |
| [ ] | `workloads.rs` (daemonsets) | `src/resources/daemonsets.ts` | none (unit only) |
| [ ] | `workloads.rs` (replicasets) | `src/resources/replicasets.tsx` | none (unit only) |
| [ ] | `workloads.rs` (replication controllers) | `src/resources/replicationcontrollers.tsx` | none (unit only) |
| [x] | `jobs.rs` | `src/resources/jobs.tsx` | `batch-a-workloads` |
| [x] | `cronjobs.rs` | `src/resources/cronjobs.tsx` | `batch-a-workloads` |
| [x] | `nodes.rs` | `src/resources/nodes.tsx` | `batch-e-cluster`, `us2-node-actions`, `node-shell` |
| [x] | `namespaces.rs` | `src/resources/namespaces.tsx` | `batch-e-cluster`, `us1-namespace-filter` |
| [x] | `events.rs` | `src/components/panels/EventsPanel.tsx` | `batch-e-cluster`, `batch-a-workloads` |
| [x] | `services.rs` | `src/resources/services.tsx` | `network-kinds`, `us1-port-forward-start` |
| [x] | `endpoints.rs` | `src/resources/endpoints.tsx` | `network-kinds` |
| [x] | `ingresses.rs` | `src/resources/ingress.tsx` | `network-kinds` |
| [x] | `ingressclasses.rs` | `src/resources/ingressclasses.tsx` | `network-kinds` |
| [x] | `networkpolicies.rs` | `src/resources/networkpolicies.tsx` | `network-kinds` |
| [x] | `persistentvolumes.rs` | `src/resources/storage.tsx` | `batch-c-storage-config` |
| [x] | `persistentvolumeclaims.rs` | `src/resources/storage.tsx` | `batch-c-storage-config` |
| [x] | `storageclasses.rs` | `src/resources/storage.tsx` | `batch-c-storage-config` |
| [x] | `roles.rs` | `src/resources/roles.tsx` | `us1-rbac-browse-roles` |
| [x] | `rolebindings.rs` | `src/resources/rolebindings.tsx` | `us1-rbac-browse-roles`, `us2-rbac-create-binding`, `us3-rbac-edit-delete-binding` |
| [x] | `clusterroles.rs` | `src/resources/clusterroles.tsx` | `us1-rbac-browse-roles` |
| [x] | `clusterrolebindings.rs` | `src/resources/clusterrolebindings.tsx` | `us1-rbac-browse-roles`, `us3-rbac-edit-delete-binding` |
| [x] | `configmaps.rs` | `src/resources/configKinds.tsx` | `batch-c-storage-config` |
| [x] | `secrets.rs` | `src/resources/configKinds.tsx` | `batch-c-storage-config` |
| [x] | `resourcequotas.rs` | `src/resources/configKinds.tsx` | `batch-c-storage-config` |
| [x] | `limitranges.rs` | `src/resources/configKinds.tsx` | `batch-c-storage-config` |
| [x] | `horizontalpodautoscalers.rs` | `src/resources/horizontalpodautoscalers.tsx` | `batch-d-config` |
| [x] | `poddisruptionbudgets.rs` | `src/resources/configKinds.tsx` | `batch-c-storage-config` |
| [ ] | `priorityclasses.rs` | `src/resources/priorityclasses.tsx` | none (unit only: `batchD.test.tsx`) |
| [ ] | `runtimeclasses.rs` | `src/resources/runtimeclasses.tsx` | none (unit only) |
| [ ] | `leases.rs` | `src/resources/leases.tsx` | none (unit only: `batchD.test.tsx`) |
| [x] | `mutatingwebhooks.rs` | `src/resources/webhooks.tsx` | `batch-d-config` |
| [ ] | `validatingwebhooks.rs` | `src/resources/webhooks.tsx` | none (unit only; shares the file with mutating) |
| [x] | `flux.rs` (Flux custom resources) | `src/components/panels/FluxPanel.tsx`, `src/resources/flux-resource.tsx` | `us1-applications-list`, `us2-application-inspector` |

## FR-002 mutating actions (each behind a confirmation)

| Done | Action | New-UI equivalent | E2E spec(s) |
|---|---|---|---|
| [x] | Delete (single and bulk) | `src/components/ConfirmDialog.tsx`, `src/resources/actions.ts` | `us3-bulk-delete` |
| [x] | Scale | `src/components/ScaleDialog.tsx` | `us3-scale` |
| [ ] | Restart (deployments, statefulsets, daemonsets, replicasets, RCs) | `src/resources/actions.ts` (`Restart<Kind>`) | none from the row menu; only Vitest (`batchA.test.tsx`) and MCP-agent restart (`us3-agent-ops-confirm`) |
| [x] | Create / stop port forward | `src/components/panels/PortForwardStartDialog.tsx`, `PortForwardsPanel.tsx` | `us1-port-forward-start`, `us2-port-forward-manage`, `us3-port-forward-port-picker` |

## FR-003 dashboards, streaming, settings

| Done | Capability | New-UI equivalent | E2E spec(s) |
|---|---|---|---|
| [x] | Overview dashboard | `src/components/panels/OverviewPanel.tsx` | `batch-e-cluster` |
| [ ] | Workload metrics charts | `src/components/panels/MetricsPanel.tsx` | `metrics-grafana` covers the Prometheus/Grafana settings; no spec asserts per-workload chart rendering (gap) |
| [ ] | Log streaming with follow | `src/components/panels/PodLogsTab.tsx` | `us1-pod-logs` covers Start/Stop and chunk rendering; no spec asserts the follow toggle / scroll-stick (gap) |
| [x] | Port-forward management | `src/components/panels/PortForwardsPanel.tsx` | `us2-port-forward-manage`, `us4-port-forward-copy-address` |
| [x] | Integrations settings | `src/components/panels/ClusterSettingsPanel.tsx`, `GrafanaPanel.tsx`, `MetricsPanel.tsx` | `metrics-grafana`, `us2-teams-cluster-settings` |
| [x] | MCP settings | `src/components/panels/ClusterSettingsPanel.tsx` | `mcp-settings-persistence`, `mcp-agent-status` |
| [x] | Status bar with backend notifications | `src/components/shell/StatusBar.tsx` | `error-states-shell` |

## Remaining `src/app/views/*.rs` files

| Done | egui view | New-UI equivalent | E2E spec(s) |
|---|---|---|---|
| [x] | `overview.rs` | `OverviewPanel.tsx` | `batch-e-cluster` |
| [x] | `integrations.rs` | `ClusterSettingsPanel.tsx`, `MetricsPanel.tsx`, `GrafanaPanel.tsx` | `metrics-grafana`, `batch-i-ai-identity-portforward` |
| [x] | `portforwards.rs` | `PortForwardsPanel.tsx` | `us2-port-forward-manage` |
| [x] | `status.rs` | `src/components/shell/StatusBar.tsx` | `error-states-shell` |
| [x] | `context.rs` | `ContextPicker.tsx`, `NamespacePicker.tsx` | `us1-context-switch`, `us1-namespace-filter` |
| [x] | `navigation.rs` | `NavTree.tsx`, `src/resources/nav.ts` | `responsive-shell`, `us1-command-palette` |
| [x] | `modal.rs` | `ConfirmDialog.tsx`, `ScaleDialog.tsx`, `PortForwardStartDialog.tsx` | `us3-scale`, `us3-bulk-delete`, `us1-port-forward-start` |
| [x] | `detail_sections.rs` | `src/components/DetailSections.tsx` | `us1-inspector` |
| [ ] | `placeholder.rs` | `src/components/panels/UnavailableSection.tsx` | none found by grep (gap 6) |
| n/a | `mod.rs`, `traits.rs` | `src/resources/index.ts`, `types.ts` (descriptor registry) | structural; covered by `tests/unit/descriptors.test.ts` |

## Gaps (block sign-off; T124 and therefore T125 stay open)

Missing E2E, each a small spec following `batch-a-workloads.spec.ts` (open the section, assert rows and an inspector):

1. StatefulSets, DaemonSets, ReplicaSets, ReplicationControllers (FR-001).
2. PriorityClasses, RuntimeClasses, Leases, ValidatingWebhookConfigurations (FR-001).
3. Row-menu Restart with confirmation (FR-002).
4. Workload metrics chart rendering in `MetricsPanel` (FR-003).
5. Log follow toggle in `PodLogsTab` (FR-003).
6. UnavailableSection (placeholder for unsupported sections) has no E2E.

Not verified by this checklist: that no egui-only behavior exists beyond what the view files name; WebKit runs (the Playwright WebKit browser here lacks system libs, so only Chromium was executed by the author of this checklist).
