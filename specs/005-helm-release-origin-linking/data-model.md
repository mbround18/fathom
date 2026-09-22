# Data Model: Helm Release View & Origin-Context Linking

## Entities

### Helm Release

The logical entity behind Helm's release-storage Secrets/ConfigMaps (one release = latest
revision's storage object, plus its revision history of prior storage objects sharing the same
release name/namespace).

`HelmReleaseSummary` (list row):

```rust
pub struct HelmReleaseSummary {
    pub name: String,
    pub namespace: String,
    pub chart: String,            // chart name, e.g. "nginx-ingress"
    pub chart_version: String,    // chart's own version, e.g. "4.10.0"
    pub app_version: Option<String>,
    pub revision: i64,            // current revision number
    pub status: HelmReleaseStatus,
    pub updated: String,          // RFC3339, from the release's `info.last_deployed`
}
```

`HelmReleaseStatus` mirrors Helm's own `status` field verbatim (no reinterpretation):

```rust
pub enum HelmReleaseStatus {
    Deployed, Failed, PendingInstall, PendingUpgrade, PendingRollback,
    Superseded, Uninstalled, Uninstalling, Unknown,
}
```

`HelmReleaseDetails` (Inspector detail):

```rust
pub struct HelmReleaseDetails {
    pub name: String,
    pub namespace: String,
    pub chart: String,
    pub chart_version: String,
    pub app_version: Option<String>,
    pub revision: i64,
    pub status: HelmReleaseStatus,
    pub updated: String,
    pub description: Option<String>,       // Helm's info.description (e.g. "Upgrade complete")
    pub values_user_supplied: String,      // YAML text, release.config (empty doc if none)
    pub values_effective: String,          // YAML text, chart.values merged with config
    pub manifest: String,                  // YAML text, release.manifest verbatim
    pub revisions: Vec<HelmRevisionSummary>,
    pub origin: Option<GitOpsOriginRef>,
    pub related: Vec<RelatedResourceRef>,  // owned workloads, via "owns" (data-model reuse from 004)
    pub degraded: bool,                    // true if release payload failed to decode (FR-014)
}
```

**Validation / degraded-row rule (FR-014)**: if the release-storage object's data key exists but
fails to base64-decode, gunzip, or JSON-parse, the release still appears using only its storage
object's own labels/metadata (`name`, `namespace`, `status`, `version` label → `revision`), with
`degraded: true` and `values_*`/`manifest` left as empty strings — never a hard error that drops
the row from the list.

### Release Revision

One prior (or current) revision's storage object, list-only (no diffing — spec.md Assumption).

```rust
pub struct HelmRevisionSummary {
    pub revision: i64,
    pub status: HelmReleaseStatus,
    pub description: Option<String>,
    pub updated: String,           // RFC3339
    pub app_version: Option<String>,
    pub chart_version: String,
}
```

`HelmReleaseDetails.revisions` is sorted descending by `revision`, built by listing every
release-storage object sharing `{name, namespace}` (not just the latest), decoding each just far
enough to read its own status/description/timestamp (full manifest/values are only decoded for the
currently-displayed revision, not every historical one, to avoid unnecessary gunzip/parse work on
revisions the operator isn't looking at).

### GitOps Origin Reference

Generic origin-source pointer (spec.md's explicit forward-compatibility requirement — must not be
Flux-specific in shape, even though Flux is the only source implemented in v1).

```rust
pub struct GitOpsOriginRef {
    pub controller: GitOpsController,   // enum: Flux (v1; Argo reserved, not implemented)
    pub section: SectionKey,            // routes to the origin's own inspector (section_keys::flux_resource())
    pub kind: String,                   // "HelmRelease" | "Kustomization" (Flux's own kind)
    pub namespace: String,
    pub name: String,
    pub ready: FluxReadyState,          // resolved live via SelectFluxResource (research.md R3)
    pub message: Option<String>,
}
```

`GitOpsController` is a one-variant enum today (`Flux`) rather than a bare string, so a future
`Argo` variant is additive, not a breaking rename (research.md's forward-compatibility note).

`HelmReleaseDetails.origin` is `None` for a bare-Helm release with no Flux ownership markers
(`helm.toolkit.fluxcd.io/name`/`namespace` labels absent) — an absent link, not an error state, per
spec.md's explicit non-goal framing.

### Flux Resource (origin detail target)

The minimal read-only detail shown when an origin link is opened (research.md R4). Reuses the
existing `FluxResourceSummary` protocol type from `src/protocol/v1/flux.rs` unchanged — no new
struct, just a new command/event pair to fetch one on demand (contracts/protocol.md).

## Relationships

- **Helm Release → owned workloads**: `"owns"` relation (existing vocabulary from 004), resolved
  via Helm's own tracking labels (`app.kubernetes.io/managed-by=Helm`, `meta.helm.sh/release-name`,
  `meta.helm.sh/release-namespace`) instead of `ownerReferences` (Helm-installed resources
  typically have no owner reference back to the release, since a release is not itself a
  Kubernetes object with a UID — this is why label-based attribution, already built in
  `applications.rs`, is reused rather than the `ownerReferences`-based `related_for_workload`
  path).
- **Helm Release → GitOps origin**: new `"reconciled-by"` relation (research.md R3), one-directional
  (a Helm release points at the Flux object that reconciles it; the reverse direction — Flux
  object → the releases it manages — is out of scope for this spec, since Flux's own inspector view
  is deliberately minimal, R4).

## State / Lifecycle

No new state machine — `HelmReleaseStatus` mirrors Helm's own status values directly; Fathom does
not derive or transition state itself (read-only, FR-015).
