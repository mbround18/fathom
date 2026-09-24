---

description: "Task list for Kubernetes Cluster Version Badge (019)"
---

# Tasks: Kubernetes Cluster Version Badge

**Input**: Design documents from `/specs/019-cluster-version-badge/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Explicitly requested by the task brief (round-trip serde tests, one hyper-focused
Playwright e2e). Included below.

**Organization**: Tasks are grouped by user story (US1 = server version display, US2 = skew
warning) per spec.md priorities.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1 or US2, per spec.md

## Phase 1: Setup

- [x] T001 Register the new backend module: add `mod cluster_version;` to `src/backend.rs`
  (alongside the existing `mod flux;` / `mod nodes;` declarations, keeping alphabetical order).
- [x] T002 Register the new protocol module: add `mod cluster_version;` and
  `pub use cluster_version::*;` to `src/protocol/v1/mod.rs` (alphabetically near `clusterroles`).

## Phase 2: Foundational (blocking prerequisites for both user stories)

**Purpose**: The protocol type, the backend fetch/poll plumbing, and the store wiring are shared
by both user stories (US2 is additive fields on the same type/event, not a separate one) — build
once here.

- [x] T003 Create `src/protocol/v1/cluster_version.rs` defining `ClusterVersionSummary` per
  data-model.md: `server_version: Option<String>`, `node_kubelet_versions: Vec<String>` (distinct,
  sorted — data-model.md invariant), `skew_detected: bool` (MUST be `false` whenever
  `server_version` is `None` or `node_kubelet_versions` is empty — data-model.md invariant).
  `#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]`.
- [x] T004 Add `ClusterVersionSummary` to the `use super::{...}` import list in
  `src/protocol/v1/event.rs`, and add the `ClusterVersionUpdated(ClusterVersionSummary)` variant to
  `enum SystemEvent` (per contracts/protocol.md), with a doc comment referencing 019.
- [x] T005 Create `src/backend/cluster_version.rs` with
  `pub(super) async fn fetch_cluster_version(client: &kube::Client) -> anyhow::Result<ClusterVersionSummary>`:
  - Call `client.apiserver_version()` (same call as `context_status.rs::probe_context`) to get
    `server_version` (its `git_version` field); on error/timeout, set `server_version: None`
    rather than propagating the error (spec FR-005 — neutral "unknown" state), following R1/R4 in
    research.md.
  - List nodes the same way `src/backend/nodes.rs::fetch_node_summaries` does
    (`Api::<Node>::all(client.clone()).list(&ListParams::default())`), extract
    `status.node_info.kubelet_version` per node (same field `nodes.rs::system_info()` reads), and
    build `node_kubelet_versions` as a deduplicated, sorted `Vec<String>` (empty on
    error/no-nodes-visible, not propagated as an error — spec Edge Cases).
  - Compute `skew_detected` per the data-model.md invariant (only `true` when `server_version` is
    `Some` AND at least one `node_kubelet_versions` entry differs from it).
- [x] T006 Add `cluster_version_poll_task: Option<tokio::task::JoinHandle<()>>` to the
  `WatchTasks` struct in `src/backend/state.rs`, initialize it to `None` alongside the other
  `_poll_task` fields, and add it to `WatchTasks::abort_all()` (mirrors `flux_poll_task` /
  `hubble_poll_task` exactly, so a context switch aborts and restarts this poll like the others).
- [x] T007 Add `ensure_cluster_version_poll(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>)`
  and `poll_cluster_version(...)` to `src/backend/sync.rs`, mirroring
  `ensure_flux_poll`/`poll_flux_overview` exactly (per contracts/protocol.md): guard on
  `state.watch_tasks.cluster_version_poll_task.is_some()`, spawn a tokio task that does an
  immediate `fetch_cluster_version` call before entering the 20s `tokio::time::interval` loop (per
  research.md R4, so the badge populates promptly on connect instead of waiting up to 20s), emits
  `BackendEvent::System(SystemEvent::ClusterVersionUpdated(summary))` via
  `Route::global(section_keys::system())` (per research.md R5) on success, and calls
  `events::notify_error(...)` on failure without tearing down the loop.
- [x] T008 Call `sync::ensure_cluster_version_poll(state, sender).await;` from
  `refresh_core_resources` in `src/backend/cmd/cluster.rs`, alongside the existing
  `ensure_flux_poll`/`ensure_hubble_poll` calls.
- [x] T009 [P] Add `sample_cluster_version_summary()` to `tests/protocol_serde.rs` (per
  contracts/protocol.md: `server_version: Some("v1.29.4")`,
  `node_kubelet_versions: vec!["v1.28.9", "v1.29.4"]`, `skew_detected: true`), call it from the
  `system_roundtrip` test list with `SystemEvent::ClusterVersionUpdated(sample_cluster_version_summary())`,
  and add `SystemEvent::ClusterVersionUpdated(..) => {}` to the `all_system_events_is_exhaustive`
  match (this match fails to compile without it, by design).
- [x] T010 [P] Add a `clusterVersion` slice to the frontend store shape in `ui/src/store/index.ts`
  (holding `ClusterVersionSummary | undefined`, `undefined` before the first event arrives — per
  research.md R4) and a `case 'ClusterVersionUpdated': d.put('clusterVersion', event.data); return`
  arm in `ui/src/store/handlers/system.ts` (mirrors the existing
  `case 'FluxOverviewUpdated': d.put('flux', { overview: event.data })` pattern one-for-one, noting
  `ClusterVersionSummary` is the whole slice value here rather than nested under an `overview`
  key).

**Checkpoint**: `cargo build`, `cargo test --test protocol_serde`, and `pnpm -C ui typecheck`
(bindings regenerated) should all pass before starting either user story below.

## Phase 3: User Story 1 - See the cluster's Kubernetes version at a glance (P1)

**Goal**: A persistent badge near the cluster/context switcher shows the connected cluster's
Kubernetes server version, updating on context switch, with a neutral state when unknown.

**Independent Test**: Connect Fathom to a cluster (or inject a `ClusterVersionUpdated` fixture
event against the fake backend) and verify a badge showing the server version renders near the
cluster/context switcher, without opening any other pane.

- [ ] T011 [US1] Create `ui/src/components/shell/ClusterVersionBadge.tsx`: reads
  `clusterVersion` from the store; renders `null` (nothing) only if the app has no connected
  context at all (mirrors `FederationContextSwitcher`'s `if (contexts.length === 0) return null`
  early-return convention), otherwise renders a small pill: the version string (e.g. `v1.29.4`)
  when `server_version` is `Some`, or a neutral "Unknown" label/muted style when `undefined`
  (before first event) or `server_version` is `None` (spec FR-005 / Acceptance Scenario 3). Use
  `data-testid="cluster-version-badge"` on the container and `data-server-version` /
  `data-skew-detected` attributes on it for Playwright assertions (mirrors
  `FederationContextSwitcher`'s `data-context-status` convention).
- [ ] T012 [US1] Mount `<ClusterVersionBadge />` beside `<FederationContextSwitcher />` in
  `src/components/shell/AppShell.tsx` (per plan.md's Project Structure), so it renders at every
  width class alongside the existing always-visible context switcher strip (Constitution IV — no
  pane-count regression, present across width classes).
- [ ] T013 [P] [US1] Add a `clusterVersion` fixture batch builder (or inline `system(seq, {...})`
  calls, following `tests/fixtures/batches.ts`'s existing pattern for `federationContexts`) to
  `ui/tests/fixtures/batches.ts` for a `ClusterVersionUpdated` event with `server_version:
  "v1.29.4"`, `node_kubelet_versions: ["v1.29.4"]`, `skew_detected: false`.
- [ ] T014 [US1] Create `ui/tests/e2e/uc-cluster-version-badge.spec.ts` (mirrors
  `us1-federation-context-status.spec.ts`'s structure: `open(page, 'manual')`, `emit(page,
  clusterVersionHealthy)`, then `expect(page.getByTestId('cluster-version-badge')).toHaveAttribute('data-server-version',
  'v1.29.4')` and assert the visible text contains `v1.29.4`) covering US1's Acceptance Scenario 1
  (badge shows server version) and Scenario 3 (badge shows "Unknown" neutral state when no event
  has arrived yet / `server_version` is `null`).

**Checkpoint**: US1 is independently functional and testable — `pnpm -C ui test:e2e -- uc-cluster-version-badge`
passes for the non-skew assertions.

## Phase 4: User Story 2 - Be warned about control-plane/node version skew (P2)

**Goal**: The badge shows a distinct warning indicator when any node's kubelet version differs
from the control-plane server version, with the specific differing version(s) inspectable via
hover/click, and no warning when everything matches.

**Independent Test**: Inject a `ClusterVersionUpdated` fixture event with `skew_detected: true`
and multiple `node_kubelet_versions` entries; verify the warning indicator renders and the
distinct versions are visible on hover/click. Inject one with `skew_detected: false`; verify no
warning renders.

- [ ] T015 [US2] Extend `ClusterVersionBadge.tsx` (from T011) to render a visible warning
  indicator (icon/dot with a distinct color, e.g. amber, following
  `FederationContextSwitcher`'s `DOT_CLASS` Record-lookup convention) when `skew_detected` is
  `true`, and no warning element at all when `skew_detected` is `false` or the slice is
  `undefined` (spec Acceptance Scenarios 1 and 4 — no false warning before data loads).
- [ ] T016 [US2] Add a lightweight hover tooltip / click popover to `ClusterVersionBadge.tsx`
  (per spec FR-004) listing the distinct `node_kubelet_versions` alongside the `server_version`
  when `skew_detected` is `true`, using `title={...}` for the simple hover case (mirrors
  `FederationContextSwitcher`'s `title={...}` tooltip convention) plus a `data-testid="cluster-version-skew-details"`
  element containing the version list text for Playwright assertion (an always-hoverable `title`
  attribute alone is not reliably assertable in Playwright, so the visible/DOM detail element is
  the primary test hook).
- [ ] T017 [P] [US2] Add a second fixture batch (or inline event) to
  `ui/tests/fixtures/batches.ts` for a skewed `ClusterVersionUpdated` event: `server_version:
  "v1.29.4"`, `node_kubelet_versions: ["v1.28.9", "v1.29.4"]`, `skew_detected: true`.
- [ ] T018 [US2] Extend `ui/tests/e2e/uc-cluster-version-badge.spec.ts` (from T014) with
  assertions covering US2's Acceptance Scenarios 1-3: emit the skewed fixture and assert the
  warning indicator (`data-skew-detected="true"`) is present and the skew-details element lists
  `v1.28.9`; then emit the healthy fixture from T013 and assert the warning indicator is gone
  (`data-skew-detected="false"` or absent).

**Checkpoint**: Both user stories pass their e2e assertions together in
`uc-cluster-version-badge.spec.ts`; `pnpm -C ui test:e2e -- uc-cluster-version-badge` is green.

## Phase 5: Polish & Cross-Cutting Concerns

- [ ] T019 [P] Run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; fix any
  warnings introduced by the new `cluster_version` modules.
- [ ] T020 [P] Run `pnpm -C ui lint` and fix any warnings in the new `ClusterVersionBadge.tsx` /
  store changes.
- [ ] T021 Regenerate TS bindings (ts-rs) so `ui/src/protocol/generated/ClusterVersionSummary.ts`
  and the updated `SystemEvent.ts` exist, per the project's existing binding-generation step (same
  step already used for `FluxOverview.ts`); confirm no other generated files spuriously changed.
- [ ] T022 Run the full verification suite: `cargo build`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, and the new e2e spec; confirm
  all pass before commit.

## Dependencies

- Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1) → Phase 4 (US2) → Phase 5 (Polish).
- Phase 4 (US2) depends on Phase 3's `ClusterVersionBadge.tsx` component (T011) existing, since
  US2 extends it in place rather than creating a new component — so US2 is not fully independent
  of US1's component scaffold, though its *fixture data and skew logic* are additive-only and do
  not require T011's US1-specific rendering to be "done" first, only present.
- T009 (protocol serde test) and T010 (store wiring) both depend on T003/T004 (the type/event
  existing) but not on each other — parallelizable.
- T013 and T017 (fixture batches) are parallelizable with each other and with backend tasks (T005-T008),
  since they only depend on the protocol type shape (T003), not the backend implementation.

## Parallel Execution Examples

- After Phase 2's T003/T004 land, T009 (Rust protocol test) and T010 (frontend store wiring) can
  proceed in parallel — different files, no shared dependency beyond the type definition.
- T013 and T017 (fixture batches, different event payloads in the same file but additive/non-conflicting
  entries) can be drafted in parallel with T005-T008 (backend fetch/poll implementation), since
  e2e fixtures only need the protocol shape, not the live backend.
- T019 (`cargo fmt`/`clippy`) and T020 (`pnpm lint`) are independent and parallelizable in Phase 5.

## Implementation Strategy

**MVP scope**: Phase 1 + Phase 2 + Phase 3 (User Story 1 only) delivers the core value — an
always-visible server-version badge — and is independently shippable/demoable per spec.md's
priority ordering (P1 alone is a viable increment). User Story 2 (skew warning) is a strictly
additive enhancement on the same component and event, layered in Phase 4.

Recommended order: Phase 1 → Phase 2 → Phase 3 (checkpoint: US1 demoable) → Phase 4 (checkpoint:
full feature demoable) → Phase 5 (checkpoint: CI-green, ready to commit).
