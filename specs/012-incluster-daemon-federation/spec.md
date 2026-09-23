# Feature Specification: Multi-Cluster Context Federation & Optional In-Cluster Daemon

**Feature Branch**: `012-incluster-daemon-federation`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the multi-cluster context federation and optional in-cluster daemon feature per docs/MVP.md §14 Tier D. Extend Fathom's single-active-kubeconfig-context model to: (P1) an always-visible multi-context switcher with live per-context connection status; (P2) true multi-cluster federation with simultaneous live connections and a cross-cluster aggregate view; (P3) native credential/auth-plugin plumbing for EKS/GKE/AKS via the official vendor exec-credential plugins; (P4) an explicitly optional in-cluster daemon that enriches — but is never required for — cluster connectivity."

## User Scenarios & Testing *(mandatory)*

<!--
  IMPORTANT: User stories should be PRIORITIZED as user journeys ordered by importance.
  Each user story/journey must be INDEPENDENTLY TESTABLE - meaning if you implement just ONE of them,
  you should still have a viable MVP (Minimum Viable Product) that delivers value.
-->

### User Story 1 - See and switch between every cluster context at a glance (Priority: P1)

An operator who manages several clusters (dev, staging, prod, or clusters across teams) wants to see every context from their kubeconfig in one persistent list, know at a glance which ones Fathom can currently reach, and switch the active cluster with a single click instead of hunting through a hidden dropdown.

**Why this priority**: This is the foundation every other capability in this feature builds on. Today Fathom already loads all context names and lets the operator pick one, but the picker is not a first-class, always-visible surface and gives no indication of reachability before the operator commits to switching. Making the existing single-context reality visible as a list with live status is valuable on its own and ships independently of any multi-cluster aggregation work.

**Independent Test**: Can be fully tested by opening Fathom against a kubeconfig with multiple contexts (including at least one that is unreachable, e.g. points at a stale IP) and confirming every context appears in a persistent list showing its live connection status, with no scenario where the operator must open a separate dialog to discover a context exists.

**Acceptance Scenarios**:

1. **Given** a kubeconfig with three contexts ("dev", "staging", "prod"), **When** the operator opens Fathom, **Then** all three contexts appear in a persistent, always-visible list (not a modal or hidden dropdown), each showing a live status of connected, connecting, error, or not connected.
2. **Given** the operator is connected to "dev" and clicks "staging" in the list, **When** the switch completes, **Then** "staging" becomes the active context (same reconnect behavior Fathom already performs today), the list updates to show "staging" as connected and "dev" as not connected, and all resource views refresh to reflect "staging".
3. **Given** a context "prod" whose API server is unreachable, **When** the operator views the context list, **Then** "prod" shows an error status with the underlying connection error available on inspection (e.g. via hover/click), without the operator having to attempt a switch first to discover the failure.
4. **Given** the kubeconfig changes on disk (a context is added or removed) while Fathom is running, **When** Fathom next reloads kubeconfig contexts (consistent with its existing reload behavior), **Then** the list reflects the updated set of contexts without requiring an app restart.

---

### User Story 2 - Get a rolled-up view across multiple live clusters (Priority: P2)

An operator who has selected several contexts as "active" wants Fathom to keep live connections (watches) to all of them simultaneously, and to see a single aggregated overview — for example, resource counts and health rolled up per cluster — without manually switching the active context back and forth to check each one.

**Why this priority**: This is the actual "federation" capability implied by the feature name, but it depends on User Story 1's context list existing first, and it is a materially larger backend change (moving from one live client to a set of live clients). It is scoped as P2 so the MVP (P1) can ship and be validated before this heavier lift begins.

**Independent Test**: Can be fully tested by marking two or more reachable contexts as simultaneously active, confirming Fathom maintains live watches against both at once (verifiable by triggering a change in either cluster and observing it reflected without switching), and confirming the aggregate overview shows correct per-cluster rollups (e.g. pod counts, health) for each active cluster at the same time.

**Acceptance Scenarios**:

1. **Given** contexts "dev" and "staging" are both reachable, **When** the operator marks both as simultaneously active, **Then** Fathom establishes and maintains live connections to both, and a cross-cluster overview shows per-cluster resource counts and aggregated health for each, updating independently as each cluster's state changes.
2. **Given** two contexts are simultaneously active, **When** one of them becomes unreachable (e.g. network drop), **Then** the aggregate overview shows that cluster's status as errored while continuing to show live data for the still-reachable cluster, with no interruption to the working connection.
3. **Given** the operator has not explicitly marked any additional context as active, **When** they use Fathom normally, **Then** behavior is unchanged from today (exactly one live connection) — simultaneous multi-cluster connections are opt-in, never automatic.
4. **Given** several contexts are simultaneously active, **When** the operator is idle, **Then** Fathom's idle CPU usage stays within its existing budget scaled reasonably to the number of active connections, and the UI remains responsive under the same input-latency targets Fathom already guarantees for a single cluster.

---

### User Story 3 - Connect to managed-cloud clusters using standard vendor credentials (Priority: P3)

An operator whose kubeconfig context points at an EKS, GKE, or AKS cluster — and therefore relies on an exec-based credential plugin (`aws-iam-authenticator`/`aws eks get-token`, `gke-gcloud-auth-plugin`, or `kubelogin`) already configured the same way `kubectl` uses it — wants Fathom to connect using that same plugin, with no separate cloud login flow to configure inside Fathom.

**Why this priority**: This unlocks federation across real-world managed-cloud fleets, which is likely the dominant use case for multi-cluster operators, but it is additive on top of User Stories 1 and 2 (a context that already resolves to a static token or client-cert works today) and depends on nothing else in this feature, so it is sequenced after the core switcher and federation mechanics.

**Independent Test**: Can be fully tested by pointing Fathom at a kubeconfig context that specifies an exec-credential plugin for each of EKS, GKE, and AKS, with the corresponding vendor CLI installed and already working for `kubectl`, and confirming Fathom connects successfully using the same plugin invocation, and separately confirming a clear, actionable error appears when the plugin is missing.

**Acceptance Scenarios**:

1. **Given** a kubeconfig context configured with an EKS exec-credential entry (`aws eks get-token` or `aws-iam-authenticator`) that already works for `kubectl`, **When** the operator selects that context in Fathom, **Then** Fathom connects successfully using the same plugin invocation, with no additional cloud-specific login step inside Fathom.
2. **Given** the same is true for a GKE context using `gke-gcloud-auth-plugin` and an AKS context using `kubelogin`, **When** the operator selects each in turn, **Then** each connects successfully via its respective official plugin.
3. **Given** a context requires `gke-gcloud-auth-plugin` but it is not installed or not on `PATH`, **When** the operator selects that context, **Then** Fathom shows a clear, actionable error naming the missing plugin ("gke-gcloud-auth-plugin") and which cloud/context it is required for, rather than a generic connection failure.
4. **Given** a plugin-derived credential/token is obtained successfully, **When** the operator views any Fathom UI surface (logs, connection status, exported diagnostics), **Then** the raw token/credential value is never shown or logged, consistent with how Fathom already treats other credentials.

---

### User Story 4 - Optionally enrich a cluster with a lightweight in-cluster daemon (Priority: P4)

An operator who wants richer or lower-latency context than external polling can provide — for example, watches that survive Fathom disconnecting and reconnecting, or controller-level events — chooses to deploy an optional, lightweight Fathom daemon into that cluster, and Fathom's federated view for that cluster transparently uses it when present, while every cluster without the daemon continues to work exactly as it does today.

**Why this priority**: This is the most exploratory, highest-effort item in the feature and is explicitly optional enrichment rather than a requirement. It is sequenced last so the zero-in-cluster-footprint model that makes Fathom adoptable with no prerequisites is never put at risk by, or blocked on, this capability.

**Independent Test**: Can be fully tested two ways: (a) confirming a cluster with no daemon deployed behaves identically to today's direct-`kube-rs`-only connection (no behavior change, no daemon dependency introduced), and (b) deploying the optional daemon into a test cluster and confirming Fathom's federated view for that cluster picks it up and reflects daemon-sourced data (e.g. a watch that persists across a deliberate Fathom disconnect/reconnect) without any manual reconfiguration beyond acknowledging the daemon is present.

**Acceptance Scenarios**:

1. **Given** a cluster has no Fathom daemon deployed, **When** the operator connects to it (via P1/P2/P3), **Then** Fathom behaves exactly as it does today — direct API-server connection, zero in-cluster footprint required.
2. **Given** the operator deploys the optional daemon into a cluster, **When** Fathom detects it, **Then** the operator is shown that enrichment is available for that cluster and must explicitly opt in before Fathom starts using it — it is never used silently.
3. **Given** a cluster is using the daemon for enrichment, **When** the operator disconnects and reconnects Fathom (e.g. closes and reopens the app), **Then** state the daemon was maintaining (e.g. a long-lived watch) survives the gap, and the operator sees this reflected as fresher/more complete data than the direct-connection-only path would have provided.
4. **Given** a cluster's daemon becomes unreachable while Fathom is otherwise connected to that cluster's API server directly, **When** this happens, **Then** Fathom falls back to direct API-server access for that cluster without losing connectivity, and clearly indicates the enrichment is temporarily unavailable.

---

### Edge Cases

- What happens when the operator switches the single active context (P1) while a P2 federation set is also active? The P1 "active context" (the one driving the primary single-cluster views) and P2's "simultaneously federated set" are related but distinct: switching the primary active context does not implicitly add or remove clusters from the federated set, and vice versa.
- What happens when two contexts in the kubeconfig point at the same cluster (e.g. same API server, different auth)? Each is treated as its own context entry with its own connection status; Fathom does not attempt to de-duplicate clusters by API server identity.
- What happens when a context's exec-credential plugin (P3) succeeds but the resulting credential is rejected by the API server (e.g. expired IAM role, revoked RBAC)? Fathom surfaces the API server's authentication/authorization error verbatim, distinct from a "plugin not found" error, so the operator knows whether the problem is local (missing plugin) or remote (rejected credential).
- What happens when the operator marks more contexts as simultaneously active (P2) than the system can comfortably sustain within its performance budget? Fathom does not silently degrade one connection to protect another; it surfaces a clear signal (e.g. a warning) that the active set may be exceeding practical limits, while leaving the decision to disconnect any context to the operator.
- What happens if the in-cluster daemon (P4) is present but running an incompatible/older protocol version than the connected Fathom client? Fathom treats this the same as "daemon unreachable" — it falls back to direct API-server access and indicates the version mismatch rather than sending or accepting data it cannot safely interpret.
- What happens to per-context connection state (which contexts are simultaneously active, whether daemon enrichment is opted into) across an app restart? It is persisted the same way Fathom already persists other operator preferences, and restoring it re-establishes connections rather than requiring the operator to redo their selections — but restoring never bypasses the explicit-opt-in requirement of Principle I (state is restored because the operator previously and explicitly chose it, not silently expanded).

## Requirements *(mandatory)*

### Functional Requirements

**Context visibility & switching (P1)**

- **FR-001**: The system MUST display every context found in the loaded kubeconfig in a persistent, always-visible list rather than a menu the operator must open to discover contexts exist.
- **FR-002**: The system MUST show a live connection status per context — at minimum: connected, connecting, error, and not connected — and MUST update that status without requiring the operator to manually refresh.
- **FR-003**: The system MUST allow the operator to select any listed context to make it the single active context, using the same reconnect behavior already implemented today, with all resource views refreshing to reflect the newly active context.
- **FR-004**: When a context is in an error state, the system MUST make the underlying connection error available to the operator (e.g. on inspection) without requiring them to first attempt switching to that context.
- **FR-005**: The system MUST reflect additions or removals of contexts in the kubeconfig on the same reload cadence/triggers Fathom already uses for detecting kubeconfig context changes, without requiring an app restart.

**Multi-cluster federation (P2)**

- **FR-006**: The system MUST allow the operator to explicitly mark more than one context as simultaneously active, distinct from and in addition to the single "active context" used by primary single-cluster views.
- **FR-007**: For every context marked simultaneously active, the system MUST establish and maintain its own live, independent connection (including watches), such that data from one active cluster is never blocked, delayed, or dropped by another active cluster's connection state.
- **FR-008**: The system MUST provide an aggregated, cross-cluster overview showing per-cluster rollups (at minimum: resource counts and an aggregated health/status indicator) for every simultaneously active context.
- **FR-009**: If a simultaneously active context becomes unreachable, the system MUST reflect that cluster's degraded/error state in the aggregate overview while continuing uninterrupted service for the remaining reachable clusters.
- **FR-010**: Simultaneous multi-cluster connections MUST be strictly opt-in per context; the system MUST NOT automatically establish live connections to every context in the kubeconfig.
- **FR-011**: The system MUST keep idle resource usage and input responsiveness within Fathom's existing performance budgets, scaled reasonably to the number of simultaneously active contexts, and MUST warn the operator rather than silently degrade when the active set appears to exceed practical limits.

**Cloud auth-plugin plumbing (P3)**

- **FR-012**: For a kubeconfig context that specifies an exec-based credential plugin, the system MUST invoke that plugin using the same mechanism already used by standard Kubernetes tooling (i.e., it MUST reuse the official vendor plugin rather than reimplementing the cloud's token exchange).
- **FR-013**: The system MUST support, at minimum, the exec-credential paths for EKS (`aws-iam-authenticator` / `aws eks get-token`), GKE (`gke-gcloud-auth-plugin`), and AKS (`kubelogin`).
- **FR-014**: When a context's required exec-credential plugin is not installed or not resolvable, the system MUST surface a clear, actionable error naming the specific missing plugin and the context/cloud it is required for, distinguishable from a credential-rejected-by-the-API-server error.
- **FR-015**: The system MUST NOT log, export through the protocol to any display surface, or show in the UI the raw token/credential values obtained from an exec-credential plugin, unless the operator explicitly requests to reveal them (consistent with how Fathom already treats other credentials).

**Optional in-cluster daemon (P4)**

- **FR-016**: The system MUST function with zero in-cluster footprint by default; no capability delivered by this feature may require the in-cluster daemon to be present for Fathom to connect to and operate against a cluster.
- **FR-017**: When a cluster's optional daemon is present and detected, the system MUST require an explicit operator opt-in before using it for enrichment; it MUST NOT silently switch from direct API-server access to daemon-mediated access.
- **FR-018**: When an opted-in daemon becomes unreachable, the system MUST fall back to direct API-server access for that cluster without losing connectivity, and MUST clearly indicate to the operator that enrichment is temporarily unavailable.
- **FR-019**: The daemon and its protocol MUST reuse Fathom's existing Rust backend/protocol crates rather than introducing a new, separate wire format or technology stack.

**Cross-cutting**

- **FR-020**: All new per-context and per-cluster connection state introduced by this feature (status, simultaneously-active membership, daemon opt-in) MUST flow through Fathom's versioned command/event protocol; the UI MUST NOT read or infer this state by any means outside that protocol.
- **FR-021**: This feature MUST NOT introduce any new resource-mutating capability; connecting, switching, marking contexts active/inactive, and opting into daemon enrichment are connection-management actions, not cluster mutations, and existing mutation gates (confirmation naming affected objects) are unaffected by this feature.
- **FR-022**: Persisted state introduced by this feature (which contexts are simultaneously active, daemon opt-in per cluster) MUST follow Fathom's existing rules for on-disk state: it MUST keep loading across upgrades, and unreadable/invalid persisted state MUST fall back to defaults without deleting the operator's existing file.

### Key Entities

- **Kubeconfig Context**: An entry from the loaded kubeconfig (name, cluster, user/credential reference). Already partially modeled today via `ContextsUpdated`; this feature adds a live per-context connection-status attribute and membership in the simultaneously-active set.
- **Active Context**: The single context driving Fathom's primary, non-aggregated resource views today — unchanged in meaning by this feature, but now displayed within the always-visible context list rather than a separate picker.
- **Federated Context Set**: The operator-selected subset of contexts marked simultaneously active for cross-cluster aggregation (P2). Distinct from, and not required to include, the Active Context.
- **Cluster Connection**: A live client/watch session for one context, with a status (connected/connecting/error/not connected), an optional last-error detail, and — for P4 — an optional link to that cluster's opted-in daemon enrichment state.
- **Exec-Credential Plugin Reference**: The vendor plugin (name, invocation) a context's kubeconfig `exec` block specifies, used to obtain short-lived credentials; not persisted or displayed beyond identifying which plugin is required.
- **In-Cluster Daemon Link**: Per-cluster state describing whether an optional daemon was detected, whether the operator has opted into using it, and its current reachability — never required for a cluster's baseline connection.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator managing 5+ clusters can identify which are currently reachable and switch the active cluster in under 5 seconds, without opening any view beyond the persistent context list.
- **SC-002**: An operator can mark 3 reachable clusters as simultaneously active and see correct, independently-updating per-cluster data in the aggregate overview within 10 seconds of marking them active, with no cluster's data blocked by another's connection state.
- **SC-003**: 100% of connection failures (unreachable context, missing exec-credential plugin, rejected credential) surface a specific, distinguishable error to the operator — never a generic or silent failure.
- **SC-004**: An operator with a working EKS/GKE/AKS `kubectl` setup can connect Fathom to the same cluster with no additional cloud-specific configuration beyond selecting the context, on the first attempt.
- **SC-005**: A cluster with no optional daemon deployed shows zero behavioral difference from Fathom's pre-existing single-cluster connection experience, in 100% of cases exercised.
- **SC-006**: With 3 simultaneously active clusters and the app otherwise idle, CPU usage and UI input latency remain within Fathom's existing single-cluster budgets scaled proportionally, with no case of the UI becoming unresponsive.

## Assumptions

- Fathom already parses and lists kubeconfig contexts (`src/backend/kube.rs`, `ContextsUpdated`) and already performs a single-context connect/reconnect cycle; this feature extends that existing mechanism rather than replacing it.
- "Simultaneously active" (P2) is an explicit, operator-driven selection separate from the single "active context" used by today's primary views; the two concepts can overlap (the active context may also be part of the federated set) but neither implies the other.
- The exec-credential mechanism already supported by the underlying Kubernetes client library covers invoking external plugins per the standard kubeconfig `exec` spec; this feature's job is ensuring Fathom's packaging/environment lets that mechanism find and run the official vendor plugins (`aws-iam-authenticator`/`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`) that the operator has already installed for `kubectl`, not building a new credential-exchange mechanism.
- No new authentication protocol is introduced for the optional in-cluster daemon beyond what Fathom's existing protocol crates already define; daemon discovery and enrichment are treated as an additive channel, not a parallel identity system.
- There is no fixed numeric cap on how many contexts may be simultaneously active; the system instead surfaces a warning when practical performance limits are approached, leaving the decision to the operator (consistent with Principle I's explicit-operator-control stance).
- This feature targets the existing supported platforms (Ubuntu 26.04, Windows 11, macOS 26) with no new platform-specific behavior, other than relying on the operator's own OS-level installation of the relevant cloud vendor CLI/plugin for P3.
- RBAC/OIDC identity and role-scoped UI (docs/MVP.md §14 Tier C) and shared team connection profiles (§14 Tier F) are explicitly out of scope here and are expected to build on top of this feature's context/connection model in separate specs.
