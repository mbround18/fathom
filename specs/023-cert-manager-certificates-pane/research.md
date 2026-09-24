# Phase 0 Research: Certificates (cert-manager) Pane

## R1: CRD discovery mechanism

**Decision**: Reuse the exact discovery technique in `src/backend/flux.rs`: list all
`CustomResourceDefinition` objects once per scan, find a served version for each cert-manager CRD
name by literal `plural.group` match, then read instances via `kube::core::DynamicObject` +
`ApiResource::from_gvk`. No `cert-manager` Rust crate is added.

**Rationale**: This is the established, version-agnostic pattern already used for Flux and
Longhorn. cert-manager has shipped `cert-manager.io/v1` as its only served version for years, but
using discovery (rather than hardcoding `v1`) costs nothing and stays consistent with the rest of
the codebase, and tolerates a cluster that has only `v1alpha2`/`v1alpha3` (legacy, still seen in
the wild) without extra code paths.

**Alternatives considered**: A dedicated `cert-manager` API client crate — rejected, violates the
"no new external dependency" constraint and the constitution's dependency-stability preference.
Hardcoding `cert-manager.io/v1` GVKs — rejected in favor of discovery for consistency with
`flux.rs`, at negligible extra cost (one extra CRD list already happens every scan).

## R2: Which CRDs gate "installed"

**Decision**: `installed = true` when the `Certificate` CRD (`certificates.cert-manager.io`) is
present and served. The other four (`CertificateRequest`, `Order`, `Challenge`,
`Issuer`/`ClusterIssuer`) are read opportunistically for the chain drill-down and issuer name
resolution, but their absence does not flip `installed` to false — some minimal installs might not
have run any ACME flow yet (no Orders/Challenges exist at all), which is a valid healthy state, not
a "not installed" state.

**Rationale**: Matches spec FR-001/FR-002: cert-manager is "installed" iff its core CRD group is
present; distinguishing "not installed" from "installed but empty" hinges on the CRD that always
exists in any cert-manager install — `Certificate`.

**Alternatives considered**: Requiring all 6 CRDs present — rejected, unnecessarily strict and
would misclassify a healthy fresh install with no in-flight ACME orders as "not installed".

## R3: Chain correlation (Certificate → CertificateRequest → Order → Challenge)

**Decision**: Correlate purely through the owner references and labels cert-manager itself sets,
matching the spec's Assumptions:
- Certificate → CertificateRequest: cert-manager labels every CertificateRequest it creates for a
  Certificate with `cert-manager.io/certificate-name=<certificate-name>` in the same namespace.
  Among matches, pick the one with the newest `metadata.creationTimestamp` (spec Edge Cases: "most
  recent/relevant" on retries).
- CertificateRequest → Order: cert-manager's ACME issuer sets an owner reference from the Order to
  the CertificateRequest (`metadata.ownerReferences[].kind == "CertificateRequest"`); find the
  Order whose owner reference UID matches the CertificateRequest's UID.
- Order → Challenge(s): Challenges carry an owner reference back to their Order the same way;
  collect all Challenges owned by the selected Order (usually one, but multi-domain certs can have
  more — the "stuck" stage picks the least-advanced Challenge among them).

**Rationale**: This is exactly how `kubectl cert-manager status certificate` and the
`cmctl status` tooling correlate the chain, and it requires no new CRD watches beyond listing the
four kinds already in scope — reads already return `metadata.ownerReferences` and
`metadata.labels` on `DynamicObject` for free.

**Alternatives considered**: Correlating by name-prefix conventions (cert-manager historically
generates predictable suffixed names) — rejected as fragile/undocumented behavior; owner
references and the documented `cert-manager.io/certificate-name` label are the stable, intended
correlation mechanism.

## R4: "Stuck stage" classification

**Decision**: Walk the chain in order (Certificate → CertificateRequest → Order → Challenge) and
report the *first* stage that is not in a terminal-success state as the stuck stage:
- Certificate: stuck if its `Ready` condition is `False` and no further chain progress exists.
- CertificateRequest: stuck if `Ready`/`Approved` conditions are `False` or the CertificateRequest
  itself failed validation before an Order exists.
- Order: stuck if `status.state` is not `valid` (e.g. `pending`, `errored`, `invalid`).
- Challenge: stuck if `status.state` is not `valid` (e.g. `pending`, `errored`, `invalid`).

If the Certificate's `Ready` condition is `True`, no stage is "stuck" — the chain view shows the
most recent completed CertificateRequest (if it or its descendants still exist) purely for
context, per spec Acceptance Scenario 2 (US2).

**Rationale**: Directly implements spec FR-007; this mirrors the mental model an operator already
uses when running `kubectl describe challenge` by hand — walk conditions/state fields, stop at the
first non-ready link.

**Alternatives considered**: Reporting all non-ready stages — rejected; spec explicitly asks for
"exactly where it's stuck" (singular), and multiple flagged stages would be more confusing, not
less.

## R5: Expiry countdown computation

**Decision**: Compute `notAfter - now` from `status.notAfter` (RFC3339, always set once a
Certificate is issued at least once) on the backend, and send both the raw timestamp and a
pre-classified state (`Valid`, `ExpiringSoon`, `Expired`, `Unknown`) using a 14-day default
threshold (per spec Assumptions), so the frontend never needs its own clock-skewed "now" for
classification — only for live re-rendering of the countdown text between poll ticks (a cheap,
non-network `Date.now()` diff against the fixed `notAfter` timestamp already sent).

**Rationale**: Backend-classified state avoids duplicating threshold logic in two languages and
keeps classification testable with plain `cargo test` unit tests (Constitution III: "UI logic that
decides what the operator sees ... MUST have unit tests" — here the decision is made once, in Rust,
and the UI's job is only to render a state it's told, reducing UI-side test surface to formatting).

**Alternatives considered**: Sending only the raw timestamp and classifying entirely in the UI —
rejected, duplicates domain logic and drifts from the single-source-of-truth pattern the rest of
the app follows (backend owns interpretation of cluster state, e.g. `FluxReadyState::classify`).

## R6: Live update mechanism

**Decision**: A 20-second interval poll task (`poll_cert_manager_overview`), started via
`ensure_cert_manager_poll` the same way `ensure_flux_poll`/`ensure_hubble_poll` are — best-effort,
idempotent, aborted/restarted alongside the other watch tasks in `WatchTasks::abort_all`. The chain
drill-down (`SelectCertificateChain`) is an on-demand read triggered when the operator selects a
Certificate, not a second poll loop — mirrors `SelectFluxResource`'s one-shot read pattern.

**Rationale**: A poll (not a `kube::runtime::watcher`) is consistent with Flux/Hubble and
sufficient for spec SC-004 ("same live-update latency window as the existing Flux panel"). Adding
a fifth long-lived watch stream for a read-only, non-critical-path pane is unjustified complexity.

**Alternatives considered**: A `watcher()`-based live stream for instant updates — rejected as
disproportionate; the spec's own success criterion pins latency parity with Flux, which uses
polling, not watching.
