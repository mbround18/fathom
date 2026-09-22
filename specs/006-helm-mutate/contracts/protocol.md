# Protocol Contract: Helm Mutate (006)

New additions to `src/protocol/v1/cmd.rs` (`BackendCmdV1`) and `src/protocol/v1/event.rs` (`SystemEvent`), alongside the existing read-only Helm variants (`RefreshHelmReleases`, `SelectHelmRelease`) from 005. Types referenced (`HelmMutationAction`, `HelmMutationPreviewReady`, `HelmMutationApplied`, `HelmMutationFailed`, `HelmMutationStage`) are defined in `data-model.md` / `src/protocol/v1/helm.rs`.

## BackendCmdV1 additions

```rust
/// Compute a rollback diff-preview without mutating anything (FR-002).
/// mutation_target() -> Namespaced { namespaces: vec![namespace] }
PreviewHelmRollback {
    namespace: String,
    release: String,
    target_revision: i64,
},

/// Apply a previously previewed rollback. `base_revision` MUST match the
/// preview's `base_revision`; a mismatch fails with HelmMutationFailed
/// (stage: Apply) per the stale-preview edge case (FR-012).
/// mutation_target() -> Namespaced { namespaces: vec![namespace] }
ApplyHelmRollback {
    namespace: String,
    release: String,
    target_revision: i64,
    base_revision: i64,
},

/// Compute an upgrade diff-preview via `helm upgrade --dry-run` without
/// mutating anything (FR-004). `values_yaml` is the (possibly edited)
/// effective values; `chart_version` is None to keep the current version.
/// mutation_target() -> Namespaced { namespaces: vec![namespace] }
PreviewHelmUpgrade {
    namespace: String,
    release: String,
    values_yaml: String,
    chart_version: Option<String>,
},

/// Apply a previously previewed upgrade. `base_revision` MUST match the
/// preview's `base_revision` (stale-preview check, FR-012).
/// mutation_target() -> Namespaced { namespaces: vec![namespace] }
ApplyHelmUpgrade {
    namespace: String,
    release: String,
    values_yaml: String,
    chart_version: Option<String>,
    base_revision: i64,
},

/// Uninstall a release. No preview step exists (spec.md US3 rationale — nothing
/// to preview). `base_revision` guards against uninstalling a release the
/// operator's confirm dialog is stale against.
/// mutation_target() -> Namespaced { namespaces: vec![namespace] }
UninstallHelmRelease {
    namespace: String,
    release: String,
    base_revision: i64,
},
```

## SystemEvent additions

```rust
HelmMutationPreviewReady(HelmMutationPreviewReady),
HelmMutationApplied(HelmMutationApplied),
HelmMutationFailed(HelmMutationFailed),
```

## Round-trip test coverage (Constitution Principle III)

Per 003/005's established convention in `tests/protocol_serde.rs`, every one of the 5 new `BackendCmdV1` variants and 3 new `SystemEvent` variants gets a `roundtrip(...)` serde test, following the exact pattern already used for `ApplyResourceYaml`, `SelectHelmRelease`, etc. (see `cmd.rs:664-785` for the existing style to match). This is a `tasks.md` Foundational-phase item, not implemented in this plan document.

## Frontend consumption contract

- `ui/src/resources/helm.tsx`'s Inspector gains three action entry points (Rollback on a revision row, Upgrade, Uninstall), each dispatching the relevant `Preview*`/`UninstallHelmRelease` command.
- `HelmMutationPreviewReady` is rendered through a generalized version of 003's diff-view component (accepts `current_manifest`/`target_manifest` strings directly instead of live-vs-edited-buffer state) plus, when `gitops_warning` is present, a warning banner reusing 005's origin-link rendering from `helm.tsx` (same link-out affordance, new warning copy per FR-008).
- Confirm step (rollback/upgrade apply, and uninstall) reuses `ui/src/components/ConfirmDialog.tsx` unmodified, naming the single release by namespace/name (R3 in research.md) — when a `gitops_warning` is present it is shown inside this same dialog, above the confirm action.
- The frontend never receives raw Helm release-storage bytes and never talks to the cluster directly (FR-011) — every payload above is already-processed strings/structs from the backend.
