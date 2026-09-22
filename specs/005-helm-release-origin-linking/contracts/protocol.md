# Protocol Contract: Helm Release View & Origin-Context Linking

All new variants are read-only (Constitution Principle I trivially satisfied — no mutation.rs
changes, per FR-015). Follows the exact shape 004 used for `applications.rs` (see
`specs/004-applications-interlinking/contracts/protocol.md` for the precedent this mirrors).

## New module: `src/protocol/v1/helm.rs`

Exports `HelmReleaseSummary`, `HelmReleaseDetails`, `HelmReleaseStatus`, `HelmRevisionSummary`,
`GitOpsOriginRef`, `GitOpsController` — exact field shapes in `data-model.md`. Each derives
`Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type` (or this codebase's equivalent
derive set — match whatever `applications.rs` uses verbatim) so `ts-rs`/specta regenerates the
matching `ui/src/protocol/generated/*.ts` files.

## `src/protocol/v1/route.rs` — new `SectionKey` constructors

```rust
pub fn helm_releases() -> SectionKey { SectionKey::new("helm/releases") }
pub fn flux_resource() -> SectionKey { SectionKey::new("integrations/flux-resource") }
```

`flux_resource()` is distinct from the existing `integrations_flux()` (the dashboard aggregate) —
it is the per-object origin-detail target (research.md R4).

## `src/protocol/v1/cmd.rs` — new `BackendCmdV1` variants

```rust
// Helm releases (read-only)
RefreshHelmReleases,
SelectHelmRelease { namespace: String, name: String },

// Flux single-resource lookup (read-only; distinct from RefreshFlux's dashboard aggregate)
SelectFluxResource { kind: String, namespace: String, name: String },
```

`RefreshRelatedResources { section, kind, namespace, name }` (existing, unchanged shape) gains a
new dispatch target: `kind == "HelmRelease"` in `related_resources()` (contracts below) — no new
command variant needed for owned-resources, reusing 004's existing generic resolver command.

Round-trip test additions in `tests/protocol_serde.rs`, mirroring
`application_commands_roundtrip`/`refresh_related_resources_roundtrip`:

```rust
#[test]
fn helm_release_commands_roundtrip() { /* RefreshHelmReleases, SelectHelmRelease */ }

#[test]
fn select_flux_resource_roundtrip() { /* SelectFluxResource */ }
```

## `src/protocol/v1/event.rs` — new event variants

```rust
// In ResourceListEvent
HelmReleasesUpdated(Vec<HelmReleaseSummary>),

// In ResourceReadEvent
HelmReleaseDetailsUpdated(HelmReleaseDetails),
FluxResourceDetailsUpdated(FluxResourceSummary),   // reuses existing flux.rs protocol type
FluxResourceNotFound { kind: String, namespace: String, name: String },  // FR-013
```

Round-trip test additions, mirroring `ApplicationsUpdated`/`ApplicationDetailsUpdated`:

```rust
#[test]
fn helm_release_events_roundtrip() { /* HelmReleasesUpdated, HelmReleaseDetailsUpdated */ }

#[test]
fn flux_resource_events_roundtrip() { /* FluxResourceDetailsUpdated, FluxResourceNotFound */ }
```

## `src/backend/helm.rs` (new)

```rust
pub async fn list_releases(client: &Client, namespace: Option<&str>) -> anyhow::Result<Vec<HelmReleaseSummary>>;
pub async fn get_release_details(client: &Client, namespace: &str, name: &str) -> anyhow::Result<HelmReleaseDetails>;

// Internal — release-storage decode (unit-testable without a live cluster, FR/Constitution III)
fn decode_release_payload(base64_gzip_json: &str) -> anyhow::Result<ReleasePayload>;
```

`decode_release_payload` unit tests use a fixture-encoded blob (base64+gzip+JSON of a small
synthetic release object), per the plan-prompt's explicit instruction — no live cluster required,
mirroring how 004 tested `attribute_from_meta` without a client.

## `src/backend/related.rs` — dispatcher extension

```rust
match kind {
    // ...existing arms unchanged...
    "HelmRelease" => related_for_helm_release(state, namespace, name).await,
    _ => Ok(Vec::new()),
}
```

`related_for_helm_release` returns owned-workload `"owns"` relations (data-model.md) plus, when an
origin exists, one `"reconciled-by"` relation pointing at `section_keys::flux_resource()` with
`kind` = `"HelmRelease"` or `"Kustomization"` as resolved from the release's Flux labels.

## `src/backend/cmd/` — dispatch wiring

- `RefreshHelmReleases` → `helm::list_releases` → `ResourceListEvent::HelmReleasesUpdated`, routed
  via `Route::namespaced(section_keys::helm_releases(), ctx, ns)` (per the 004 postmortem — always
  `namespaced`, never `global`, for a namespaced section).
- `SelectHelmRelease` → `helm::get_release_details` → `ResourceReadEvent::HelmReleaseDetailsUpdated`.
- `SelectFluxResource` → re-run `flux::list_kustomizations_and_helmreleases` + `summarize()`
  (existing `flux.rs` functions, research.md R3), filter for the matching
  `{kind, namespace, name}` → `ResourceReadEvent::FluxResourceDetailsUpdated` on a match, else
  `FluxResourceNotFound` (FR-013).

## Frontend — `ui/src/resources/`

- `helm.tsx`: `ResourceDescriptor<HelmReleaseSummary, HelmReleaseDetails>` for
  `section_keys::helm_releases()`. `read()` issues `SelectHelmRelease` + `RefreshRelatedResources`
  (kind `"HelmRelease"`). `detailSections`: metadata, values (user-supplied + effective, each via
  the existing YAML viewer from 003), manifest (same viewer), revisions (list block), related
  (existing `relatedResourcesBlock` from 004, unchanged). No `rawJson` (mirrors Applications'
  precedent — a release is not a single fetchable/appliable k8s object).
- `flux-resource.tsx`: `ResourceDescriptor<FluxResourceSummary, FluxResourceSummary>` for
  `section_keys::flux_resource()` — one "Details" fields block (Ready state, message, revision,
  suspended, last_transition), reusing `DetailSections.tsx`'s existing `fields` block type. `read()`
  issues `SelectFluxResource`.
- `index.ts`: append both descriptors to `ALL_DESCRIPTORS`.
- `nav.ts`: add a `"Helm Releases"` entry to `NAV_GROUPS` pointing at `section_keys::helm_releases()`
  (the Flux-resource section is not nav-visible — it's only reachable via an origin link, matching
  how CRD-instance detail sections work today).
