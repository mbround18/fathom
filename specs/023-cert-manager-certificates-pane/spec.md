# Feature Specification: Certificates (cert-manager) Pane

**Feature Branch**: `023-cert-manager-certificates-pane`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Certificates (cert-manager) pane — detect cert-manager via its CRDs (cert-manager.io API group: Certificate, CertificateRequest, Order, Challenge, Issuer/ClusterIssuer), show a read-only tool-pane listing Certificates with expiry countdown and renewal status, and let the operator drill into a failing/pending certificate's lifecycle chain (Certificate → CertificateRequest → Order → Challenge) to see exactly where it's stuck and why. Read-only, no mutation surface, no new external dependency. Non-goals: no certificate issuance/renewal-triggering actions, no ACME account management UI."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See certificate expiry at a glance (Priority: P1)

An operator opens the Certificates pane on a cluster that has cert-manager installed and immediately sees every Certificate in the cluster with its expiry countdown (e.g. "expires in 12 days") and current renewal/ready status, so they can spot certificates that are expiring soon or already failed without hunting through raw YAML.

**Why this priority**: Expiry blindness is the single most common cert-manager pain point reported by operators; a flat, sortable list with countdowns is the MVP that delivers the core value on its own.

**Independent Test**: Point the app at a cluster with cert-manager and several Certificate resources in varying states (healthy/long-lived, soon-to-expire, failed). Open the Certificates pane and confirm each Certificate row shows name, namespace, expiry countdown, and ready/renewal status, sorted so the most urgent certificates are easiest to find.

**Acceptance Scenarios**:

1. **Given** cert-manager is installed and a Certificate is valid with 60 days until expiry, **When** the operator opens the Certificates pane, **Then** the pane shows that Certificate with a "Ready" status and an expiry countdown of approximately 60 days.
2. **Given** a Certificate is within a configurable "expiring soon" threshold (e.g. under 14 days), **When** the operator views the pane, **Then** that Certificate is visually flagged as expiring soon and sorted near the top.
3. **Given** a Certificate's status condition is `Ready=False`, **When** the operator views the pane, **Then** the row shows a failed/not-ready status distinct from healthy and expiring-soon rows.

---

### User Story 2 - Diagnose a stuck certificate's lifecycle chain (Priority: P2)

An operator sees a failing or pending Certificate in the list and drills into it to see the full lifecycle chain (Certificate → CertificateRequest → Order → Challenge) so they can identify exactly which stage is stuck (e.g. an Order awaiting a Challenge that keeps failing DNS-01 propagation) and why, without manually correlating four different resource types by hand.

**Why this priority**: Listing certificates alone tells you something is wrong; the drill-down is what turns the pane into an actual diagnostic tool and is the differentiating value over a generic resource list.

**Independent Test**: With a Certificate stuck at a Challenge (e.g. ACME HTTP-01 validation failing), select that Certificate in the pane and confirm the drill-down view renders the chain (Certificate → CertificateRequest → Order → Challenge), highlights the Challenge as the stuck stage, and surfaces the Challenge's failure reason/message.

**Acceptance Scenarios**:

1. **Given** a Certificate has an associated CertificateRequest, Order, and Challenge, and the Challenge is not yet valid, **When** the operator drills into the Certificate, **Then** the chain view shows all four linked resources in order and marks the Challenge as the current/stuck stage with its status reason and message.
2. **Given** a Certificate is fully issued and healthy with no in-progress CertificateRequest, **When** the operator drills in, **Then** the chain view shows the most recent completed CertificateRequest (if any) without implying an in-progress failure.
3. **Given** a Certificate's CertificateRequest failed validation before an Order was created, **When** the operator drills in, **Then** the chain view shows the CertificateRequest as the stuck stage (no Order/Challenge to display) along with its failure reason.

---

### User Story 3 - Honest empty state when cert-manager isn't installed (Priority: P3)

An operator opens the Certificates pane on a cluster without cert-manager and sees a clear, honest message that cert-manager is not detected, rather than an empty list that looks like "no certificates exist" or an error.

**Why this priority**: Prevents misleading the operator into thinking a healthy cluster has zero certificates, or that the feature is broken; matches the established pattern from the Flux GitOps panel.

**Independent Test**: Point the app at a cluster with no cert-manager CRDs installed. Open the Certificates pane and confirm it shows a "cert-manager not installed" empty state rather than a blank or error view.

**Acceptance Scenarios**:

1. **Given** none of the cert-manager CRDs are present in the cluster, **When** the operator opens the Certificates pane, **Then** the pane shows a not-installed empty state explaining that cert-manager was not detected.
2. **Given** cert-manager's CRDs are present but zero Certificate resources currently exist, **When** the operator opens the pane, **Then** the pane shows a distinct "no certificates found" state (not the "not installed" state).

---

### Edge Cases

- What happens when cert-manager CRDs are installed but the operator lacks RBAC permission to list Certificates (or related chain resources) in some/all namespaces? Pane should show what is visible and indicate that restricted namespaces are excluded, consistent with existing RBAC-aware panes.
- How does the pane behave when a Certificate references a ClusterIssuer or Issuer that no longer exists? The issuer reference should still display (by name) even if it cannot be resolved, without crashing the pane.
- How does the system handle a Certificate whose chain resources (CertificateRequest/Order/Challenge) have already been garbage-collected after successful issuance (normal cert-manager cleanup behavior)? The pane must not treat "no chain resources found" for a Ready certificate as an error.
- What happens when there are multiple CertificateRequests for one Certificate (e.g. retried issuance)? The chain view should follow the most recent/relevant CertificateRequest for the current state.
- How does expiry countdown render for an already-expired certificate? It must show a clearly negative/expired state (e.g. "expired 3 days ago") rather than a negative number or blank.
- What happens when the cluster connection drops while the pane is open? The pane should indicate stale/disconnected data rather than silently freezing on the last-known state, consistent with other live panes.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST detect whether cert-manager is installed in the connected cluster by checking for the presence of its CRDs (`Certificate`, `CertificateRequest`, `Order`, `Challenge`, `Issuer`, `ClusterIssuer` in the `cert-manager.io` API group family).
- **FR-002**: System MUST show an honest "cert-manager not installed" empty state when the required CRDs are absent, distinct from a "no certificates found" state shown when CRDs exist but zero Certificates are present.
- **FR-003**: When cert-manager is detected, system MUST display a read-only list of all Certificate resources visible to the operator's current RBAC scope, including at minimum: name, namespace, associated Issuer/ClusterIssuer reference, expiry timestamp, an expiry countdown, and a ready/renewal status.
- **FR-004**: System MUST visually distinguish at least three certificate states in the list: healthy/ready, expiring soon (within a defined threshold), and failed/not-ready.
- **FR-005**: System MUST keep the Certificates list up to date as cluster state changes (via live watch/poll, matching the update behavior of the existing Flux panel) without requiring a manual refresh.
- **FR-006**: System MUST allow the operator to select a Certificate and view its lifecycle chain: the Certificate, its most relevant CertificateRequest, and (when present) the associated Order and Challenge resources.
- **FR-007**: System MUST identify and highlight which stage of the lifecycle chain is currently "stuck" (i.e., the earliest non-ready/non-succeeded stage) along with the human-readable reason/message reported by that resource's status conditions.
- **FR-008**: System MUST correctly represent a fully-issued, healthy Certificate whose intermediate chain resources have been cleaned up, without presenting this as an error or missing data.
- **FR-009**: System MUST NOT expose any action that creates, edits, deletes, or triggers renewal/issuance of any cert-manager resource — the pane and drill-down are read-only.
- **FR-010**: System MUST link Certificates and their chain resources to the existing resource Inspector so the operator can view full resource detail/YAML for any resource in the chain.
- **FR-011**: System MUST NOT introduce a new external runtime dependency (e.g. no new crate/package solely for cert-manager types beyond generic Kubernetes API/CRD access already used elsewhere in the app).
- **FR-012**: System MUST respect existing RBAC scoping so a user only sees Certificates and chain resources they are permitted to read, consistent with other resource panes.
- **FR-013**: System MUST indicate when displayed certificate data is stale (e.g. due to a lost cluster connection) rather than presenting last-known data as current.

### Key Entities

- **Certificate**: A cert-manager resource representing a desired TLS certificate; key attributes include name, namespace, issuer reference, expiry/`notAfter` time, and ready condition.
- **CertificateRequest**: Represents a single request to an Issuer to produce a certificate for a Certificate; tracks approval and readiness status.
- **Order**: An ACME-protocol resource representing an in-progress request to a certificate authority; tracks its state (pending, valid, failed) and links to one or more Challenges.
- **Challenge**: An ACME-protocol resource representing a single domain-validation challenge (e.g. HTTP-01, DNS-01); tracks validation state and failure reasons.
- **Issuer / ClusterIssuer**: Configuration resources describing the certificate authority a Certificate is issued from; referenced by name from a Certificate, scoped to a namespace (Issuer) or cluster-wide (ClusterIssuer).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can identify all certificates expiring within a chosen threshold across the cluster in under 10 seconds of opening the pane, without inspecting any raw YAML.
- **SC-002**: An operator can determine which specific lifecycle stage a stuck certificate is blocked at, and the reported reason, within two clicks/interactions from the Certificates list.
- **SC-003**: 100% of clusters without cert-manager installed show the honest not-installed empty state rather than an empty or misleading certificate list.
- **SC-004**: The pane reflects a certificate's status change (e.g. newly failed Challenge, successful renewal) within the same live-update latency window as the existing Flux panel, with no manual refresh required.
- **SC-005**: Zero mutation-capable controls are present anywhere in the Certificates pane or drill-down view.

## Assumptions

- The existing Flux GitOps panel's detect-then-show pattern (CRD-presence detection, live poll/watch, honest not-installed empty state, linking to the resource Inspector) is the structural precedent to follow for consistency and is reused rather than redesigned.
- "Expiring soon" threshold defaults to 14 days if not otherwise configured, matching common cert-manager/Kubernetes ecosystem conventions (e.g. cert-manager's default renewal window is close to this).
- Chain resource association (Certificate → CertificateRequest → Order → Challenge) is inferred through existing owner references / labels that cert-manager itself sets on these resources, not through a bespoke correlation heuristic.
- Only the single most relevant (typically most recent) CertificateRequest is shown as "the" active chain per Certificate at a given time; historical CertificateRequests remain inspectable via the Inspector but are not enumerated in the primary drill-down.
- No new external dependency is needed because the app already has generic typed/dynamic CRD read access sufficient to model cert-manager.io types, matching the Flux panel's approach.
- RBAC scoping and stale-connection indication reuse existing app-wide mechanisms rather than introducing pane-specific new ones.
