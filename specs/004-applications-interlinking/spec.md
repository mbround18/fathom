# Feature Specification: Applications View & Rich Interlinking

**Feature Branch**: `004-applications-interlinking`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Applications view plus the generic cross-resource drill-down/interlinking mechanism it depends on, per docs/MVP.md §5 and §5b. Aggregate workloads into logical applications from Helm-release labels and Flux Kustomization/HelmRelease inventories; show a list and a read-only inspector with rolled-up metrics and related resources. Every resource inspector with a natural parent/child or peer relationship must surface those relationships as clickable links with a breadcrumb/back-stack. Read-only only — no Helm release parsing, no topology graph, no new mutations, no Argo CD."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See my applications at a glance (Priority: P1)

An operator opens Fathom against a cluster running several Helm-installed and Flux-managed workloads and wants a single list of "applications" instead of having to mentally group Deployments, StatefulSets, Services, and ConfigMaps by hand.

**Why this priority**: This is the entry point for every other capability in this feature. Without a working application list, there is nothing to drill into and no relationships to surface.

**Independent Test**: Can be fully tested by opening the new "Applications" section in the navigation and confirming every Helm release and Flux-managed Kustomization/HelmRelease in the connected cluster/namespace scope appears exactly once, grouped correctly, with accurate namespace, managed-by, version, age, and status columns.

**Acceptance Scenarios**:

1. **Given** a cluster with a Helm-installed release "redis" in namespace "cache" and a Flux HelmRelease "web" in namespace "prod", **When** the operator opens the Applications section, **Then** both "redis" and "web" appear as separate rows with their correct namespace, a "Managed By" value of Helm or Flux respectively, and a status that reflects the aggregated readiness of their owned workloads.
2. **Given** the currently selected namespace filter excludes "cache", **When** the operator views the Applications list, **Then** "redis" is not shown, consistent with how every other resource list in Fathom already respects the namespace filter.
3. **Given** an application's owned workloads are all ready, **When** the operator views its row, **Then** the status column shows a healthy/ready indicator; **given** at least one owned workload is not ready, **then** the status column reflects a degraded/not-ready state.

---

### User Story 2 - Inspect one application in depth (Priority: P1)

An operator selects a single application from the list to understand its overall health, resource footprint, and what it's actually made of, without having to separately open each Deployment, Pod, and Service that belongs to it.

**Why this priority**: The list alone (User Story 1) only tells the operator applications exist; the inspector is where they get enough information to decide whether to act (e.g., go look at a failing pod). It's equally foundational and ships in the same first slice.

**Independent Test**: Can be fully tested by selecting an application row and confirming the right-hand Inspector opens (no page/route change) showing metadata, rolled-up CPU/memory metrics, and a related-resources list that matches the workloads actually owned by that application in the cluster.

**Acceptance Scenarios**:

1. **Given** an application "web" with 3 pods across 1 Deployment, **When** the operator selects "web" from the list, **Then** the Inspector opens in place (the list stays visible) and shows created time, current status, version (chart version or Kustomization revision), managed-by, and any URLs derivable from the application's owned Services/Ingresses.
2. **Given** the same application, **When** the operator views the Inspector's metrics area, **Then** it shows CPU and memory figures aggregated across the application's pods, sourced the same way other Fathom metrics views already are, and shows an explicit "unavailable" state (not an error) if no metrics source is configured.
3. **Given** the same application, **When** the operator views the Inspector's related-resources list, **Then** it lists the application's Pods (with readiness, CPU, memory), Deployments/StatefulSets, ServiceAccounts, and any CRD instances it owns, if applicable.

---

### User Story 3 - Drill down and retrace my steps (Priority: P1)

An operator, while inspecting an application, clicks through to one of its pods to check its status, then clicks further from that pod to the node it's running on, and then wants to get back to where they started without re-navigating from scratch.

**Why this priority**: This is the "rich interlinking" half of the feature. It is what makes the Applications view (and every other existing inspector) materially more useful than a flat list, and the user explicitly asked for it to be built alongside Applications rather than after.

**Independent Test**: Can be fully tested by opening any resource inspector that has a related resource (e.g., a Node inspector showing its scheduled Pods), clicking a related-resource link, confirming the linked resource's own inspector opens, and confirming a "back" affordance returns to the previous inspector in the same state it was left in.

**Acceptance Scenarios**:

1. **Given** the operator is inspecting application "web", **When** they click one of its related Pods, **Then** that Pod's own inspector opens (Details/YAML/Events/Logs/Terminal tabs, as already exist for pods today) and a breadcrumb shows the path taken (e.g. "web > pod-xyz").
2. **Given** the operator has drilled Application → Deployment → Pod, **When** they click "back" twice, **Then** they land first on the Deployment inspector, then on the Application inspector, each restored rather than re-fetched from a blank state.
3. **Given** the operator is inspecting a Node, **When** they view its related resources, **Then** they see the Pods currently scheduled on that node as clickable links; **given** they are inspecting a Service, **When** they view its related resources, **Then** they see the Pods it currently selects; **given** they are inspecting a PVC, **When** they view its related resources, **Then** they see the Pod(s) that mount it.
4. **Given** the operator is inspecting a Pod that is part of a recognized application, **When** they view its related resources, **Then** they see a link back up to the owning Application (in addition to any existing Pod relationships Fathom already shows, such as its Node).

---

### Edge Cases

- What happens when a workload has Helm release labels but Flux also manages it (Flux-managed HelmRelease)? The application must be classified once (Flux takes precedence, since Flux is the actively reconciling authority) and must not appear as two separate rows.
- What happens when a workload has neither Helm nor Flux labels/ownership, so it can't be attributed to any application? It is simply omitted from the Applications list — it continues to appear in the existing Workloads views as it does today.
- How does the system handle a related-resource link whose target has since been deleted (e.g., the operator clicks a Pod link but the pod was evicted seconds earlier)? The system MUST show a clear "no longer found" state in the target inspector rather than a blank screen or a silent no-op.
- What happens when an application has zero owned workloads currently matching (e.g., a Helm release object exists but every resource it once owned has been deleted out of band)? The application still appears in the list with a status indicating it has no active workloads, rather than being hidden or erroring.
- What happens when the breadcrumb/back-stack is asked to go back further than any drill-down occurred (e.g., the operator's very first navigation)? The back affordance is simply unavailable/disabled at the root, matching how back-navigation controls behave elsewhere in the product.
- How does the system handle a very large related-resources set (e.g., an application with 500 pods)? The related-resources list MUST remain usable (e.g., paginated or scrollable within the Inspector) rather than freezing the UI or being silently truncated without indication.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide a top-level "Applications" navigation section, consistent in placement and interaction with Fathom's other top-level sections (list pane + right-hand Inspector, no separate route/page).
- **FR-002**: The system MUST synthesize an "application" for every Helm release detectable via standard Helm ownership labels on live cluster resources, and for every Flux-managed Kustomization or HelmRelease already tracked by Fathom's existing GitOps integration, without requiring any new external integration.
- **FR-003**: When a workload is attributable to both a Helm release label and an actively-reconciling Flux HelmRelease, the system MUST classify it as Flux-managed and MUST NOT list it twice.
- **FR-004**: The Applications list MUST display, per application: an instance/name identifier, the application name, namespace, how it is managed (Helm, Flux, or label-only heuristic), a version indicator (chart version or GitOps revision, when known), age, and an aggregated readiness/status indicator across its owned workloads.
- **FR-005**: The Applications list MUST respect the same namespace-filter and multi-context behavior already applied to every other resource list in Fathom.
- **FR-006**: Selecting an application MUST open a read-only Inspector (no navigation away from the list) showing: creation time, aggregated status, version, managed-by, and any URLs derivable from the application's owned Services/Ingresses.
- **FR-007**: The application Inspector MUST show CPU and memory usage aggregated across the application's pods, reusing Fathom's existing metrics data source and displaying an explicit "metrics unavailable" state when no metrics source is configured, rather than an error or blank panel.
- **FR-008**: The application Inspector MUST show a related-resources list covering, at minimum, the application's Pods (with readiness, CPU, memory), Deployments/StatefulSets, ServiceAccounts, and owned CRD instances where applicable.
- **FR-009**: The system MUST provide a generic relationship-resolution capability, keyed by resource kind and its ownership/selector metadata, that determines a resource's related resources rather than each resource kind implementing its own one-off lookup.
- **FR-010**: Every existing resource inspector with a natural parent/child or peer relationship (at minimum: Application, Deployment, ReplicaSet, Pod, Node, Service, PVC) MUST surface its related resources as clickable entries, not as plain unclickable text.
- **FR-011**: Clicking a related-resource entry MUST open that resource's own inspector in place, using the same inspector experience (Details/YAML/Events/Logs/Terminal tabs, as applicable to that kind) that already exists for it today.
- **FR-012**: The system MUST maintain a breadcrumb / back-navigation stack reflecting the sequence of drill-down selections, allowing the operator to return to a previously viewed inspector.
- **FR-013**: Returning via the back-navigation stack MUST restore the previously viewed inspector without requiring the operator to reselect it from a list.
- **FR-014**: The back-navigation control MUST be unavailable/disabled when there is no prior step to return to.
- **FR-015**: When a related-resource link targets a resource that no longer exists, the system MUST show a clear "not found" state in place of that resource's inspector rather than a blank or broken view.
- **FR-016**: An application with no currently-matching owned workloads MUST still appear in the Applications list, with a status indicating it has no active workloads.
- **FR-017**: The related-resources list MUST remain usable when a resource has a very large number of related resources (e.g., scrollable/paginated), rather than freezing or silently truncating without any indication that more exist.
- **FR-018**: This feature MUST NOT introduce any new mutating capability. All drill-down navigation is read-only; any mutation available on a resource once drilled into is limited to what already exists for that resource kind today.
- **FR-019**: This feature MUST NOT read or parse Helm's release storage (secrets/configmaps holding full release manifests); Helm attribution is derived only from ownership labels already present on live cluster resources.
- **FR-020**: This feature MUST NOT include a topology graph visualization; that remains a distinct, later capability.
- **FR-021**: This feature MUST NOT add support for GitOps tools other than Flux (e.g., Argo CD is explicitly out of scope).

### Key Entities

- **Application**: A synthesized, non-Kubernetes-native grouping representing one Helm release or one Flux-managed Kustomization/HelmRelease. Attributes: name, namespace, managed-by (Helm/Flux/label-heuristic), version/revision, creation time, aggregated readiness status, and the set of resources it owns.
- **Related Resource Link**: A directional relationship between two resources (e.g., Application→Pod, Node→Pod, Service→Pod, PVC→Pod, Pod→Application) surfaced in an inspector as a clickable entry, derived from ownership references or selector matching.
- **Drill-down Breadcrumb / Back-Stack**: An ordered record of the inspectors an operator has navigated through via related-resource links within the current session, enabling backward navigation and restoration of prior inspector state.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can identify all applications running in a namespace and their overall health within 10 seconds of opening the Applications section, without needing to cross-reference any other view.
- **SC-002**: An operator can go from an application in the Applications list to a specific pod belonging to it, and back again, using only clicks within the Inspector (no manual re-search), in 3 clicks or fewer each direction.
- **SC-003**: 100% of applications present in the currently selected namespace scope (as determined by their Helm/Flux ownership metadata) appear in the Applications list; none are silently dropped.
- **SC-004**: Every related-resource entry shown across Application, Deployment, ReplicaSet, Pod, Node, Service, and PVC inspectors is clickable and leads to a live, correctly-scoped inspector for that resource.
- **SC-005**: When a related resource has been deleted between being listed and being clicked, the operator sees an explicit "not found" indication in under 2 seconds, never a blank or frozen panel.
- **SC-006**: Applications with large numbers of owned resources (hundreds of pods) remain browsable in the related-resources list without noticeable UI lag or silent truncation.

## Assumptions

- Fathom already has read access to the resource kinds needed to compute ownership/selector relationships (Pods, Deployments, ReplicaSets, Services, PVCs, Nodes, ServiceAccounts, and Flux's Kustomization/HelmRelease inventories); no new RBAC verbs are required beyond what Fathom already requests.
- "Managed By: Helm" is inferred solely from the standard `app.kubernetes.io/managed-by=Helm` and `helm.sh/release-name`/`helm.sh/release-namespace`-style labels already present on live resources — not from reading Helm's release storage objects.
- Flux's existing integration (`FluxOverviewUpdated`) already exposes enough inventory information (which resources belong to which Kustomization/HelmRelease) to attribute ownership without additional cluster calls; if a gap is found here, it is treated as a planning-time technical detail, not a scope change.
- "Version" for a Flux-managed application refers to the GitOps revision/source reference already tracked by the Flux integration; for a Helm-managed application, it refers to the chart version visible via labels/annotations already on the release's resources.
- The breadcrumb/back-stack is scoped to the current session and does not need to persist across app restarts.
- Aggregated CPU/memory metrics reuse whatever metrics source (e.g., Prometheus) is already configured; when none is configured, the existing "metrics unavailable" pattern already used elsewhere in Fathom applies here too.
- This feature targets the existing supported platforms (Ubuntu 26.04, Windows 11, macOS 26) with no new platform-specific behavior.
