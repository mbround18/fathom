# Phase 1 Data Model: Certificates (cert-manager) Pane

All types live in `src/protocol/v1/cert_manager.rs`, derive `Serialize, Deserialize, TS` (matching
`flux.rs`'s pattern), and are generated into `ui/src/protocol/generated/*.ts` by the existing ts-rs
build step.

## CertExpiryState (enum)

Backend-classified expiry bucket (Research R5).

| Variant | Meaning |
|---|---|
| `Valid` | `notAfter` is further away than the expiring-soon threshold (14 days default). |
| `ExpiringSoon` | `notAfter` is within the threshold but in the future. |
| `Expired` | `notAfter` is in the past. |
| `Unknown` | Certificate has never been issued (`status.notAfter` absent). |

## CertReadyState (enum)

Mirrors `FluxReadyState`'s shape for the per-row ready/renewal status (spec FR-004).

| Variant | Meaning |
|---|---|
| `Ready` | `Ready` condition is `True`. |
| `Renewing` | `Ready` condition is `True` but a newer in-progress CertificateRequest exists (renewal underway). |
| `Failed` | `Ready` condition is `False`. |
| `Pending` | `Ready` condition is absent/`Unknown` (initial issuance in progress). |

## CertificateSummary (struct)

One row in the Certificates list.

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | |
| `namespace` | `String` | |
| `issuer_ref` | `String` | Formatted `"Issuer/<name>"` or `"ClusterIssuer/<name>"` from `spec.issuerRef`. |
| `not_after` | `Option<String>` | RFC3339 `status.notAfter`, `None` if never issued. |
| `expiry_state` | `CertExpiryState` | |
| `expiry_seconds` | `Option<i64>` | Signed seconds until expiry (negative if expired); `None` if `not_after` is `None`. Lets the UI render a live-updating countdown without re-parsing timestamps. |
| `ready` | `CertReadyState` | |
| `message` | `String` | `Ready` condition's message (falls back to reason), same convention as `FluxResourceSummary.message`. |
| `renewal_time` | `Option<String>` | RFC3339 `status.renewalTime`, when present. |
| `dns_names` | `Vec<String>` | `spec.dnsNames`, for display context. |

## CertManagerOverview (struct)

Top-level payload for `CertManagerOverviewUpdated`.

| Field | Type | Notes |
|---|---|---|
| `installed` | `bool` | True iff the `Certificate` CRD is served (Research R2). |
| `certificates` | `Vec<CertificateSummary>` | Sorted `(namespace, name)`, same convention as Flux. |

## ChainStageKind (enum)

Identifies which link in the lifecycle chain a `ChainStage` represents: `Certificate`,
`CertificateRequest`, `Order`, `Challenge`.

## ChainStage (struct)

One resolved link in the lifecycle chain.

| Field | Type | Notes |
|---|---|---|
| `kind` | `ChainStageKind` | |
| `name` | `String` | |
| `namespace` | `String` | |
| `state` | `String` | Human label: cert-manager's own `status.state` for Order/Challenge, `Ready` condition status for Certificate/CertificateRequest. |
| `reason` | `String` | Condition/status reason, empty if none. |
| `message` | `String` | Condition/status message, empty if none. |
| `is_stuck` | `bool` | True for exactly the one stage identified by Research R4's walk (or none, if the Certificate is healthy). |

## CertificateChain (struct)

Payload for `CertificateChainUpdated` (response to `SelectCertificateChain`).

| Field | Type | Notes |
|---|---|---|
| `certificate_name` | `String` | Echoes the request, for client-side correlation. |
| `certificate_namespace` | `String` | |
| `stages` | `Vec<ChainStage>` | In chain order; omits Order/Challenge entries that don't exist (e.g. already garbage-collected after success, or never created because the CertificateRequest itself failed — spec Edge Cases). |
| `not_found` | `bool` | True if the Certificate itself could no longer be found (deleted between list and select). |

## Relationships / correlation keys

- `CertificateRequest.metadata.labels["cert-manager.io/certificate-name"] == Certificate.metadata.name`, same namespace.
- `Order.metadata.ownerReferences[].uid == CertificateRequest.metadata.uid`.
- `Challenge.metadata.ownerReferences[].uid == Order.metadata.uid`.

No new persisted entities — everything above is a live, derived read, never written to
`~/.fathom/`.
