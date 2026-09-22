# Data Model: Helm Mutate (006)

All new types live in `src/protocol/v1/helm.rs` (extending 005's module) and are derived `Serialize`/`Deserialize`/`ts-rs` (`#[derive(TS)]`) per the existing convention in that file. No existing 005 type is modified except where noted.

## HelmMutationAction

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HelmMutationAction {
    Rollback,
    Upgrade,
    Uninstall,
}
```

Identifies which of the three mutate flows a preview/apply/failure event belongs to. Carried on every new command and event so the frontend can route without inferring it from the variant name alone (useful for a single shared preview-modal component across all three actions).

## HelmMutationPreviewReady (event payload)

```rust
pub struct HelmMutationPreviewReady {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    pub base_revision: i64,          // the live revision the preview was computed against
    pub current_manifest: String,     // release's current live manifest (already known to the frontend from HelmReleaseDetails, re-sent for a self-contained preview payload)
    pub target_manifest: String,      // rollback: target revision's stored manifest verbatim. upgrade: freshly `helm upgrade --dry-run` rendered manifest.
    pub target_revision: Option<i64>, // rollback only: the revision being rolled back to
    pub target_chart_version: Option<String>, // upgrade only: chart version the upgrade would produce, if changed
    pub gitops_warning: Option<GitOpsOriginRef>, // reuses 005's type unmodified; None = no warning (FR-010)
}
```

**Validation rules** (enforced backend-side before emitting):
- `current_manifest`/`target_manifest` MUST be non-empty on success; a preview that cannot be computed emits `HelmMutationFailed` instead (never an empty-string preview).
- `target_manifest` for rollback MUST come from a successfully decoded revision (FR-012's "corrupted target revision payload" edge case fails here, before this event is ever constructed).
- `gitops_warning` is populated iff `HelmReleaseDetails.origin.is_some()` at preview time (FR-007–FR-010) — it is never independently re-derived, it is copied from the release's existing `origin` field.

Never persisted — held only in frontend memory for the duration of the diff-preview modal (Key Entity "Manifest Diff Preview" in spec.md).

## HelmMutationApplied (event payload)

```rust
pub struct HelmMutationApplied {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    pub new_revision: Option<i64>, // rollback/upgrade: the new revision number. uninstall: None (release is gone).
}
```

## HelmMutationFailed (event payload)

```rust
pub struct HelmMutationFailed {
    pub namespace: String,
    pub release: String,
    pub action: HelmMutationAction,
    pub stage: HelmMutationStage, // Preview | Apply
    pub message: String,           // verbatim backend error, per Constitution Principle I ("Backend errors MUST reach the operator verbatim")
}

pub enum HelmMutationStage { Preview, Apply }
```

Covers every FR-012 failure case: unreachable chart source / missing `helm` binary (upgrade), invalid values/template error (upgrade), corrupted target revision payload (rollback), stale base revision at confirm-time (rollback/upgrade — `Apply` stage, detected by re-checking `base_revision` against the release's live revision before applying).

## Key Entities (from spec.md, mapped to concrete shapes)

- **Helm Release Mutation Request** → the union of the five new `BackendCmdV1` variants (see `contracts/protocol.md`); not a standalone struct, expressed directly as command payloads.
- **Manifest Diff Preview** → `HelmMutationPreviewReady`, above. Never persisted (spec.md's explicit requirement).
- **GitOps Ownership Warning** → `Option<GitOpsOriginRef>` reused byte-for-byte from 005, surfaced via `HelmMutationPreviewReady.gitops_warning` and — for uninstall, which has no diff-preview step — a lighter-weight `gitops_warning: Option<GitOpsOriginRef>` field added directly to the existing `UninstallHelmRelease` confirm-dialog flow (see contracts).

## State transitions (rollback/upgrade)

```
idle → [operator picks target] → previewing (Preview* command sent)
previewing → preview_ready (HelmMutationPreviewReady received) | preview_failed (HelmMutationFailed, stage=Preview)
preview_ready → [operator confirms] → applying (Apply* command sent, carries base_revision from the preview)
preview_ready → [operator cancels] → idle (no command sent, FR-002/FR-004 "no mutation while only previewing")
applying → applied (HelmMutationApplied) | apply_failed (HelmMutationFailed, stage=Apply — includes the stale-base-revision case)
```

Uninstall has no preview stage: `idle → [operator confirms via ConfirmDialog] → applying (UninstallHelmRelease sent) → applied | apply_failed`.
