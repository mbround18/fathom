# Feature Specification: Teams & Organizations (Self-Hosted)

**Feature Branch**: `014-teams-organizations`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Teams & organizations (self-hosted) — Tier F from docs/MVP.md §14: shared cluster-connection profiles (kubeconfig/context metadata, not credentials), org-level settings sync (metrics source, hidden-metrics config from §10), and per-user RBAC-scoped views (depends on Tier C's OIDC identity + role resolution). Explicit non-goal: hosted multi-tenant SaaS control plane."

## User Scenarios & Testing *(mandatory)*

<!--
  IMPORTANT: User stories should be PRIORITIZED as user journeys ordered by importance.
  Each user story/journey must be INDEPENDENTLY TESTABLE - meaning if you implement just ONE of them,
  you should still have a viable MVP (Minimum Viable Product) that delivers value.
-->

### User Story 1 - Share a cluster-connection profile with the team (Priority: P1)

An operator who has already configured how Fathom reaches a cluster context (API server endpoint, context alias, default namespace) wants their teammates — who share the same real, self-hosted cluster — to start from that same connection profile instead of each hand-rolling the same context pointer from scratch.

**Why this priority**: This is the cheapest, highest-value slice: it has no dependency on any other future tier, requires no new identity system, and immediately removes a repeated setup chore for every team that already shares clusters. It's also the safest to ship first because it explicitly never touches credentials.

**Independent Test**: One operator publishes a connection profile for a cluster context to the team's shared config store; a second operator, pointed at the same shared store, sees that profile appear in their own Fathom install and can adopt it to add the cluster context — fully testable and valuable with no other story implemented.

**Acceptance Scenarios**:

1. **Given** an operator has an active, working cluster context in their local Fathom install, **When** they choose to share it as a team connection profile, **Then** Fathom writes only non-secret connection metadata (cluster API server endpoint, context name/alias, default namespace) to the team's shared config store, and never writes kubeconfig auth material, tokens, or credentials.
2. **Given** a teammate's Fathom install is pointed at the same shared config store, **When** they open the connection-profile list, **Then** they see the newly shared profile alongside any profiles already shared by the team.
3. **Given** a teammate sees a shared connection profile for a cluster they don't yet have configured locally, **When** they choose to adopt it, **Then** Fathom adds that cluster context using the shared endpoint/alias/namespace metadata, while the teammate supplies or points to their own kubeconfig identity to actually authenticate — no identity or credential is copied from the sharer.
4. **Given** a shared connection profile is later edited or removed by any team member with write access to the shared store, **When** other teammates' Fathom installs next sync, **Then** the change is reflected in their profile list, and a profile that was removed upstream is clearly marked as no longer shared (not silently deleted if the operator has already adopted and is actively using it).
5. **Given** the shared config store is unreachable when a teammate opens Fathom, **When** they view their connection profiles, **Then** they see their own already-adopted local profiles working normally, plus a clear, non-blocking indicator that team profile sync is unavailable — Fathom never blocks cluster access on shared-profile sync being reachable.

---

### User Story 2 - Keep org-level settings consistent across the team (Priority: P2)

An operator who has already tuned the team's shared Prometheus wiring (metrics source, Prometheus path, HTTPS toggle, path prefix) and hidden-metrics list wants that configuration to apply consistently across every teammate's Fathom install for a shared cluster, instead of each person re-entering the same values and drifting out of sync.

**Why this priority**: High value for teams already sharing clusters, and independent of Story 1 and of Tier C — but ranked below connection-profile sharing because it only matters once a team has a shared cluster context to configure settings for in the first place.

**Independent Test**: One operator changes an org-level setting (e.g. the hidden-metrics list) for a shared cluster and publishes it; a second operator's Fathom install, synced to the same shared store, reflects the updated setting without either operator re-entering it by hand — testable independent of connection-profile sharing or RBAC-scoped views.

**Acceptance Scenarios**:

1. **Given** an operator has configured Cluster Settings (metrics source, Prometheus path, HTTPS toggle, path prefix, hidden metrics) for a cluster context that is also a shared team connection, **When** they choose to publish those settings as the org-level defaults for that cluster, **Then** Fathom writes those settings (and only those settings — no other preference categories) to the shared config store.
2. **Given** a teammate's Fathom install already has local Cluster Settings for that same shared cluster context, **When** org-level settings are published or updated, **Then** the teammate is shown the incoming change and must explicitly accept it before it overwrites their local settings — it is never applied silently or optimistically.
3. **Given** a teammate has intentionally customized a cluster setting locally that differs from the org default (e.g. a personal hidden-metrics preference), **When** org-level settings sync runs, **Then** Fathom clearly distinguishes "org default" from "locally overridden" and does not clobber the local override without explicit confirmation.
4. **Given** the org-level settings received from the shared store are malformed, from an incompatible future format, or otherwise unreadable, **When** a teammate's Fathom install attempts to apply them, **Then** it falls back to the teammate's existing local settings without deleting or corrupting their local config file, and surfaces a clear error naming what failed.
5. **Given** a cluster connection is not shared with the team (a purely local, personal cluster context), **When** the operator views its Cluster Settings, **Then** no org-sync affordance is offered for it — org-level settings sync only applies to connections that are already shared team profiles.

---

### User Story 3 - See only what my role allows, per teammate (Priority: P3, gated on Tier C)

A signed-in operator on a shared team wants Fathom's UI to reflect exactly what their own Kubernetes identity can and cannot do in a given cluster — with each teammate seeing a view scoped to their own resolved role, not a single shared view for the whole install.

**Why this priority**: Highest ceiling of value for larger teams with differentiated access, but it is the most complex and the only story with an external dependency: it requires an OIDC-authenticated per-operator identity and resolved role/verb/namespace data, which do not exist yet in Fathom. This story cannot ship before that identity and role-resolution layer (tracked separately as Tier C, "Cluster-admin RBAC / OIDC-OAuth administration") exists.

**Independent Test**: Two teammates, each signed in with their own distinct OIDC identity established by Tier C and holding different resolved roles for the same cluster, open the same shared connection profile in their own Fathom installs and see different sets of enabled/visible actions matching their own role's resolved verbs and namespaces — this cannot be meaningfully tested until Tier C's identity and role-resolution data exist to drive it.

**Acceptance Scenarios**:

1. **Given** an operator is signed in via the OIDC identity established by Tier C, and Tier C has resolved that identity's roles/verbs/namespaces for the current cluster, **When** they open a shared team connection profile for that cluster, **Then** Fathom scopes the visible/enabled actions to that resolved identity's permissions, per-operator, not per-install.
2. **Given** two teammates share the same connection profile but hold different resolved roles, **When** each opens the cluster in their own Fathom install, **Then** each sees a UI scoped to their own role, independent of what the other teammate can do.
3. **Given** Tier C's identity/role-resolution layer is not yet available (current state), **When** an operator opens a shared team connection profile, **Then** Fathom does not attempt to fabricate or approximate per-user RBAC scoping, and instead presents the same install-wide view as today, with no misleading role-based UI.

---

### Edge Cases

- What happens when two teammates edit and publish conflicting connection-profile metadata for the same cluster context at nearly the same time? The shared store must surface a clear conflict rather than silently letting the last writer win without the other operator knowing their change was overwritten.
- What happens when a teammate's local kubeconfig has no context matching a shared profile's cluster endpoint at all? Adopting the profile must clearly prompt the operator to supply their own kubeconfig context/identity rather than silently failing or connecting with no identity.
- What happens when the shared config store itself is misconfigured (e.g. points at a location the operator has no access to)? Fathom must surface a clear, specific error distinguishing "can't reach the shared store" from "shared store reachable but empty."
- What happens if org-level settings publish a hidden-metrics list that references metrics that don't exist on a given teammate's cluster? Fathom must apply the parts that are valid and ignore/report the parts that don't apply, not fail the whole settings sync.
- What happens when a teammate removes team-store access (e.g. no longer points their install at the shared store)? Their already-adopted local connection profiles and settings must keep working exactly as if they'd been entered locally — losing team sync must never delete or disable existing local configuration (Constitution Principle VI).
- What happens if a shared connection profile or org-level setting is published containing what looks like a credential or secret value by mistake? Fathom must refuse to publish it and surface a clear error, rather than silently syncing it to the team.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Fathom MUST let an operator publish a cluster connection's non-secret metadata (cluster API server endpoint, context name/alias, default namespace) as a shared team connection profile to a self-hosted, team-controlled shared config store that the operator's install is already pointed at.
- **FR-002**: Fathom MUST NOT include kubeconfig authentication material, tokens, credentials, or any other secret in a published connection profile, and MUST refuse to publish a profile whose fields appear to contain such material.
- **FR-003**: Fathom MUST let an operator whose install is pointed at the same shared config store see connection profiles published by teammates, and MUST let them adopt a profile to add or update a local cluster context using its shared metadata, while requiring the operator to supply their own kubeconfig identity to authenticate.
- **FR-004**: Fathom MUST reflect updates or removals of shared connection profiles to teammates on their next sync, and MUST NOT delete a teammate's already-adopted local cluster context if the upstream shared profile is later removed — it MUST instead mark that local context as no longer team-shared.
- **FR-005**: Fathom MUST continue to function using existing local connection profiles and cluster settings when the shared config store is unreachable, and MUST show a clear, non-blocking indicator that team sync is unavailable rather than blocking cluster access.
- **FR-006**: Fathom MUST let an operator publish the existing Cluster Settings fields (metrics source, Prometheus path, HTTPS toggle, path prefix, hidden metrics) for a shared cluster connection as org-level defaults to the shared config store, and MUST NOT introduce or sync any settings category beyond those already defined for Cluster Settings.
- **FR-007**: Fathom MUST require a teammate's explicit confirmation before an incoming org-level settings change overwrites that teammate's existing local settings for a cluster — it MUST NOT apply org-level settings changes silently or optimistically (Constitution Principle I).
- **FR-008**: Fathom MUST distinguish, per setting, between "using the org-level default" and "locally overridden," and MUST preserve a teammate's local override across org-level sync unless the teammate explicitly chooses to accept the org default instead.
- **FR-009**: If org-level settings received from the shared store are unreadable, malformed, or from an incompatible format, Fathom MUST fall back to the teammate's existing local settings without deleting or corrupting the local config file, and MUST surface a clear error identifying what could not be applied (Constitution Principle VI).
- **FR-010**: Org-level settings sync MUST only be offered for cluster connections that are already shared team connection profiles; purely local, unshared cluster contexts MUST NOT display an org-sync affordance.
- **FR-011**: All persistence and synchronization of shared connection profiles and org-level settings MUST be performed by the backend through the versioned command/event protocol; the UI MUST NOT read or write the shared config store, or `~/.fathom/`, directly (Constitution Principle II).
- **FR-012**: When a shared connection profile or org-level setting is edited concurrently by two team members, Fathom MUST detect and surface the conflict to at least one of the involved operators rather than silently discarding either change.
- **FR-013** *(gated on Tier C)*: Once an OIDC-authenticated per-operator identity and resolved role/verb/namespace data are available (delivered by the separate Tier C feature), Fathom MUST scope a signed-in operator's visible and enabled actions on a shared cluster connection to that operator's own resolved role, independently of what other teammates viewing the same shared connection can do.
- **FR-014**: Until Tier C's identity and role-resolution data are available, Fathom MUST NOT attempt an approximate or partial per-user RBAC-scoped view; it MUST continue presenting today's install-wide view rather than a misleading role-based one.
- **FR-015**: A team's shared config store MUST be a self-hosted location or service the team already controls (e.g. a shared network location or a self-hosted sync mechanism); Fathom MUST NOT require or provide a Fathom-run hosted backend for this feature.

### Key Entities

- **Connection Profile**: Shareable, non-secret metadata describing how to reach one cluster context — cluster API server endpoint, context name/alias, default namespace. Owned by the team's shared config store; adopting it into a local install never carries authentication material with it. Has a lifecycle (published / adopted / updated / removed-upstream) independent per teammate's local adoption state.
- **Org-Level Cluster Settings**: The existing Cluster Settings field set (metrics source, Prometheus path, HTTPS toggle, path prefix, hidden metrics) published as a team default for one shared connection profile. Distinct from a teammate's local settings for the same cluster, which may be "following org default" or "locally overridden" per field.
- **Shared Config Store**: A self-hosted, team-controlled location or service that holds published connection profiles and org-level settings for a team. Not a Fathom-hosted service; reachability is best-effort and its unavailability must never block local cluster access.
- **Resolved Identity/Role** *(external, owned by Tier C)*: The OIDC-authenticated identity and its resolved verbs/namespaces for a cluster, consumed but not produced by this feature; Story 3 and FR-013 depend entirely on this data existing.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A teammate joining a team that already shares a cluster can go from pointing their Fathom install at the team's shared config store to having a working, authenticated connection to that cluster in under 2 minutes, without hand-entering the cluster's API endpoint or context alias.
- **SC-002**: Zero shared connection profiles or org-level settings ever contain credential or token material, verified on every publish attempt (100% of publish attempts containing secret-shaped fields are refused, not synced).
- **SC-003**: When the shared config store is unreachable, 100% of already-adopted local connection profiles and settings continue to work exactly as before, with no cluster-access interruption attributable to team-sync unavailability.
- **SC-004**: An operator's locally-overridden cluster setting is never silently overwritten by an org-level sync — 100% of org-level settings changes that would overwrite a local override require explicit confirmation first.
- **SC-005**: Once Tier C's identity/role-resolution data exists, two teammates with different resolved roles viewing the same shared connection see demonstrably different scoped actions in their respective Fathom installs (verified by comparing the two views for at least one action visible to one role and not the other).

## Assumptions

- A "team" or "org" is a group of operators who already share access to the same real, self-hosted Kubernetes cluster(s) and who point their separate Fathom installs at the same self-hosted, team-controlled shared config store (e.g. a shared network filesystem location or a lightweight self-hosted sync service) — not a Fathom-run cloud/SaaS backend. Setting up or choosing that shared store's transport (filesystem, network share, self-hosted sync service, etc.) is treated as an implementation detail for the planning phase, not a user-facing requirement of this spec.
- Each operator continues to authenticate to the cluster using their own kubeconfig identity, exactly as today; nothing in this feature changes how cluster authentication works or introduces a Fathom-native user account system.
- Story 1 (connection-profile sharing) and Story 2 (org-level settings sync) have no dependency on Tier C and can be built and shipped independently of it.
- Story 3 (per-user RBAC-scoped views) and FR-013 cannot ship before Tier C (cluster-admin RBAC/OIDC administration) delivers an OIDC-authenticated per-operator identity and resolved role/verb/namespace data; this feature consumes that data rather than building its own identity or RBAC-resolution system.
- Org-level settings sync is scoped strictly to the existing Cluster Settings fields already defined for Fathom (metrics source, Prometheus path, HTTPS toggle, path prefix, hidden metrics); it does not extend to Node Shell, Port Forward, or Preferences settings, which are out of scope for this feature.
- Conflict resolution for simultaneous edits to a shared connection profile or org-level setting favors surfacing the conflict to an operator over silently picking a winner; the exact conflict-resolution mechanism (e.g. last-write-wins with notification vs. blocking merge) is an implementation decision for the planning phase, not a user-facing requirement fixed by this spec.
- This feature does not change any existing write-action confirmation/safety model (delete, scale, restart, YAML apply, Helm mutate); publishing or adopting a shared profile or org-level setting is itself a new explicit, confirmed action following the same Principle I pattern, not a bypass of it.
