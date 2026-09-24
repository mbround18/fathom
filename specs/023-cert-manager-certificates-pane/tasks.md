---
description: "Task list for Certificates (cert-manager) pane"
---

# Tasks: Certificates (cert-manager) Pane

**Input**: Design documents from `/specs/023-cert-manager-certificates-pane/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: Included per user instruction (protocol round-trip tests, backend unit tests, Vitest
unit tests, and one focused Playwright e2e spec).

## Phase 1: Setup

- [x] T001 Add `CertExpiryState`, `CertReadyState`, `CertificateSummary`, `CertManagerOverview`,
      `ChainStageKind`, `ChainStage`, `CertificateChain` types (deriving
      `Debug, Clone, Serialize, Deserialize, TS`, matching `src/protocol/v1/flux.rs`'s style) in
      new file `src/protocol/v1/cert_manager.rs`, per data-model.md's field tables exactly
      (including the `Option<i64> expiry_seconds` and `Option<String> not_after` nullability, and
      the 4-variant `CertExpiryState`/`CertReadyState`/`ChainStageKind` enums).
- [x] T002 Re-export the new module in `src/protocol/v1/mod.rs` (`pub use cert_manager::*;` next to
      the existing `pub use flux::*;`).

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Protocol commands/events/routing and backend scan/correlation logic that every user
story's UI work depends on.

- [x] T003 Add `RefreshCertManager` and `SelectCertificateChain { namespace: String, name: String }`
      variants to `BackendCmd` in `src/protocol/v1/cmd.rs`, placed near `RefreshFlux`/
      `SelectFluxResource` with a doc comment matching contracts/protocol.md.
- [x] T004 [P] Add round-trip tests `refresh_cert_manager_roundtrip` and
      `select_certificate_chain_roundtrip` in `src/protocol/v1/cmd.rs`'s `#[cfg(test)] mod tests`,
      mirroring `select_flux_resource_roundtrip`.
- [x] T005 Add `CertManagerOverviewUpdated(CertManagerOverview)` and
      `CertificateChainUpdated(CertificateChain)` variants to `SystemEvent` in
      `src/protocol/v1/event.rs`, and add the new types to that file's `use super::{...}` import
      list.
- [x] T006 Add `C::RefreshCertManager | C::SelectCertificateChain { .. }` to the non-mutating match
      arm in `src/protocol/v1/mutation.rs` (same arm as `C::RefreshFlux | C::SelectFluxResource { .. }`).
- [x] T007 Add `section_keys::integrations_cert_manager()` and `section_keys::certificate_chain()`
      to `src/protocol/v1/route.rs` (next to `integrations_flux`/`flux_resource`), and add
      `C::RefreshCertManager => section_keys::integrations_cert_manager()` and
      `C::SelectCertificateChain { .. } => section_keys::certificate_chain()` arms to
      `mutation.rs`'s `section_for`.
- [x] T008 Create `src/backend/cert_manager.rs` modeled on `src/backend/flux.rs`:
      - CRD name constants for `certificates.cert-manager.io`, `certificaterequests.cert-manager.io`,
        `orders.acme.cert-manager.io`, `challenges.acme.cert-manager.io`,
        `issuers.cert-manager.io`, `clusterissuers.cert-manager.io`.
      - `find_served`/`list_instances` reused verbatim from (or factored out of) `flux.rs`'s
        pattern — prefer extracting the existing `find_served`/`list_instances` helpers in
        `flux.rs` into a shared internal helper module only if it can be done without changing
        `flux.rs`'s public behavior; otherwise duplicate the ~20-line helpers locally to avoid
        risky cross-module refactors of shipped code.
      - `pub(super) async fn scan_cert_manager(client: &Client) -> anyhow::Result<CertManagerOverview>`:
        `installed` iff the `Certificate` CRD (Research R2) is served; lists all Certificates,
        computes `CertificateSummary` per data-model.md (issuer_ref formatted `"Kind/name"` from
        `spec.issuerRef`, `expiry_state`/`expiry_seconds` from `status.notAfter` vs now with a
        14-day "expiring soon" threshold per research.md R5, `ready` classified per data-model.md's
        `CertReadyState` table, `message` falling back reason→message like `flux.rs::summarize`).
        Sorted by `(namespace, name)`.
      - `pub(super) async fn resolve_chain(client: &Client, namespace: &str, name: &str) -> anyhow::Result<CertificateChain>`:
        implements Research R3's owner-reference/label correlation (Certificate →
        CertificateRequest via `cert-manager.io/certificate-name` label, newest
        `creationTimestamp` on ties; CertificateRequest → Order via ownerReference UID; Order →
        Challenge via ownerReference UID) and Research R4's stuck-stage walk, returning
        `not_found: true` if the Certificate itself is missing.
- [x] T009 [P] Unit tests in `src/backend/cert_manager.rs`'s `#[cfg(test)] mod tests` (building
      `DynamicObject` fixtures by hand, no live cluster) for: expiry classification at the 14-day
      boundary (`Valid`/`ExpiringSoon`/`Expired`/`Unknown`), ready-state classification
      (`Ready`/`Renewing`/`Failed`/`Pending`), and stuck-stage classification for (a) healthy
      Certificate with no chain resources, (b) Certificate stuck at Challenge, (c) CertificateRequest
      failed before an Order existed.
- [x] T010 Add `cert_manager_poll_task: Option<tokio::task::JoinHandle<()>>` field to `WatchTasks`
      in `src/backend/state.rs`, initialize it to `None` alongside `flux_poll_task`/
      `hubble_poll_task`, and abort it in `WatchTasks::abort_all`.
- [x] T011 Add `ensure_cert_manager_poll` and `poll_cert_manager_overview` to
      `src/backend/sync.rs`, copying `ensure_flux_poll`/`poll_flux_overview`'s shape exactly
      (20s `tokio::time::interval`, emits `BackendEvent::System(SystemEvent::CertManagerOverviewUpdated(overview))`
      on `Route::cluster(section_keys::integrations_cert_manager(), context.clone())`, reports scan
      errors via `events::notify_error`). Call `ensure_cert_manager_poll` from the same call site
      that currently calls `ensure_flux_poll`/`ensure_hubble_poll` on connect.
- [x] T012 Add `RefreshCertManager`/`SelectCertificateChain` handling to
      `src/backend/cmd/integrations.rs`'s `handle` match: `refresh_cert_manager` (calls
      `cert_manager::scan_cert_manager`, emits `CertManagerOverviewUpdated`, mirrors `refresh_flux`)
      and `select_certificate_chain` (calls `cert_manager::resolve_chain`, emits
      `CertificateChainUpdated`, mirrors the `SelectFluxResource` handler in
      `src/backend/cmd/resources.rs`).
- [x] T013 Run `cargo build --workspace` to regenerate the ts-rs bindings into
      `ui/src/protocol/generated/` (`CertManagerOverview.ts`, `CertificateSummary.ts`,
      `CertExpiryState.ts`, `CertReadyState.ts`, `CertificateChain.ts`, `ChainStage.ts`,
      `ChainStageKind.ts`) and re-export them from `ui/src/protocol/types.ts` (or wherever the
      existing `FluxOverview`/`FluxResourceSummary` re-exports live).

**Checkpoint**: Backend fully detects cert-manager, lists Certificates, and resolves chains;
protocol types exist on both sides. No UI yet — foundational phase must be complete before any
user story below.

## Phase 3: User Story 1 - See certificate expiry at a glance (Priority: P1)

**Goal**: Operator opens the Certificates pane and sees every Certificate with expiry countdown
and ready/renewal status.

**Independent Test**: Scripted-backend Playwright run: emit `CertManagerOverviewUpdated` with a
healthy long-lived cert, a cert inside the expiring-soon threshold, and a `Ready=False` cert;
assert all three render with correct countdown/status.

- [x] T014 [P] [US1] Add `certManager: { overview?: CertManagerOverview; chain?: CertificateChain }`
      to `StoreData` in `ui/src/store/index.ts`, initialize `certManager: {}` in the store's
      initial state (next to `flux: {}`), and add `refreshCertManager: () => void` +
      `selectCertificateChain: (namespace: string, name: string) => void` actions implemented as
      `get().send({ type: 'RefreshCertManager' })` / `get().send({ type: 'SelectCertificateChain', namespace, name })`,
      mirroring `refreshFlux`.
- [x] T015 [US1] Handle `CertManagerOverviewUpdated` and `CertificateChainUpdated` in
      `ui/src/store/handlers/system.ts`, setting `certManager.overview` / `certManager.chain`
      respectively (mirrors the existing `FluxOverviewUpdated` case referenced in
      `ui/src/app/mailbox.rs`-equivalent TS ingest path — follow whatever existing
      `FluxOverviewUpdated`/`HubbleTrafficUpdated` case is already in `system.ts`).
- [x] T016 [P] [US1] Create `ui/src/lib/cert-expiry.ts` (or colocate in the panel file if the
      codebase has no `lib` convention for this) with a pure `formatExpiryCountdown(expirySeconds: number | null): string`
      helper: renders "expires in Nd" for positive values, "expired Nd ago" for negative values
      (per spec Edge Cases — must not show a bare negative number), and "unknown" for `null`. Add
      Vitest unit tests in `ui/src/lib/cert-expiry.test.ts` covering positive, negative, zero, and
      `null` inputs.
- [x] T017 [US1] Create `ui/src/components/panels/CertManagerPanel.tsx`: on mount, dispatch
      `refreshCertManager()` (mirrors `FluxPanel.tsx`'s `useEffect`); if `overview` is undefined,
      render `MetricsUnavailable` (waiting state); if `overview.installed === false`, render the
      not-installed empty state (US3, built here since it's a one-line branch shared with US1's
      component); otherwise render a list/table of `overview.certificates` showing name,
      namespace, issuer_ref, the `formatExpiryCountdown` output, and a status badge for `ready`
      (reuse `StatusBadge` from `@/components/StatusBadge` if its status union can be extended, or
      a local tone mapping like `FluxPanel.tsx`'s `READY_TONE`), sorted so `ExpiringSoon`/`Expired`/
      `Failed` rows sort before `Valid`/`Ready` rows (spec Acceptance Scenario 2 — "sorted near the
      top"). If `overview.installed === true && overview.certificates.length === 0`, render a
      distinct "no certificates found" message (spec US3 Acceptance Scenario 2 — do not reuse the
      not-installed copy).
- [x] T018 [US1] Register the new pane in `ui/src/components/shell/PaneHost.tsx`'s `PaneBody`
      tool switch: `if (pane.tool === 'certManager') return <CertManagerPanel />`, importing it
      next to `FluxPanel`. Add `'certManager'` to whatever `PaneTool`/`TOOL_LABEL` union/type
      definition backs `pane.tool` (find via the existing `'flux' | 'hubble' | ...` type and the
      `TOOL_LABEL` record referenced in `PaneHost.tsx`), with a label of "Certificates".

**Checkpoint**: US1 is independently functional — opening the pane shows the certificate list with
expiry/status. Ship-able as MVP on its own.

## Phase 4: User Story 2 - Diagnose a stuck certificate's lifecycle chain (Priority: P2)

**Goal**: Selecting a failing/pending Certificate shows its full chain and highlights the stuck
stage with its reason.

**Independent Test**: Scripted-backend Playwright run: select a Certificate stuck at a Challenge,
assert `SelectCertificateChain` is sent and the rendered chain marks the Challenge stage as stuck
with its message.

- [x] T019 [US2] Extend `CertManagerPanel.tsx` with row selection: clicking a Certificate row calls
      `selectCertificateChain(namespace, name)` and tracks the selected `(namespace, name)` in
      local component state, mirroring `FluxView`'s `selected` field / `FluxPanel`'s existing
      selection conventions if any exist in the React pane (check `flux-resource.tsx`'s
      click-through pattern for precedent).
- [x] T020 [US2] Add a chain drill-down section/sub-panel to `CertManagerPanel.tsx` (or a new
      `ui/src/components/panels/CertificateChainView.tsx` if the panel file would otherwise exceed
      the codebase's typical component size) that renders `certManager.chain.stages` in order as a
      simple vertical chain (Certificate → CertificateRequest → Order → Challenge), visually
      highlighting the one stage where `is_stuck === true` and showing its `reason`/`message`; if
      `certManager.chain.not_found === true`, show a "this certificate no longer exists" notice
      instead of a chain.
- [x] T021 [P] [US2] Create `ui/src/resources/certificate-resource.tsx` as a `ResourceDescriptor`
      (modeled on `ui/src/resources/flux-resource.tsx`): no navigator entry, `read` dispatches a
      resource lookup for the selected chain-stage kind, `detailSections` render the common
      fields (kind, name/namespace, state, reason, message) from `ChainStage`, and `refresh`
      re-issues `RefreshCertManager` (mirrors `flux-resource.tsx`'s `refresh: () => ({ type: 'RefreshFlux' })`
      given there's no dedicated single-object refresh command, matching contracts/protocol.md).
      Register it in whichever resource-descriptor registry `flux-resource.tsx` is registered in.
- [x] T022 [US2] Wire each `ChainStage` row in the chain view (T020) to open
      `certificate-resource.tsx`'s descriptor in the Inspector on click, using the same
      navigation call `flux-resource.tsx`'s consumers use for `RelatedResourceLink`/inspector
      hand-off (spec FR-010).

**Checkpoint**: US2 works on top of US1 — an operator can go from the list to a fully diagnosed
stuck chain, with Inspector drill-in on any link.

## Phase 5: User Story 3 - Honest empty state when cert-manager isn't installed (Priority: P3)

**Goal**: Distinct, honest empty states for "not installed" vs "installed, zero certificates".

**Independent Test**: Scripted-backend Playwright run: emit `CertManagerOverviewUpdated { installed: false, certificates: [] }`; assert the not-installed copy renders (not a blank list, not the zero-certificates copy).

Note: the not-installed and zero-certificates branches were already implemented as part of T017
(they're one conditional in the same component) — this phase is almost entirely test coverage to
lock that behavior in, per the user's explicit request for tight, hyper-focused e2e coverage.

- [x] T023 [P] [US3] Add a Vitest test for `CertManagerPanel.tsx` in
      `ui/src/components/panels/CertManagerPanel.test.tsx` (or the codebase's established test
      colocation convention) asserting: `overview === undefined` renders the waiting state,
      `overview.installed === false` renders the not-installed copy, and
      `overview.installed === true && overview.certificates.length === 0` renders the distinct
      zero-certificates copy — three separate assertions, no shared snapshot.

**Checkpoint**: All three user stories independently testable and passing.

## Phase 6: Polish & E2E

**Purpose**: The required focused Playwright coverage tying all three stories together, plus final
verification.

- [x] T024 Write `ui/tests/e2e/cert-manager-pane.spec.ts` against the project's existing
      scripted-fake-backend e2e harness (follow the setup pattern of an existing spec in
      `ui/tests/e2e/`, e.g. the Flux/Hubble e2e spec if one exists, otherwise the nearest tool-pane
      e2e spec for the harness API). Cover exactly:
      1. Open the Certificates pane against a fixture `CertManagerOverviewUpdated` with a
         Certificate close to expiry (`ExpiringSoon`) and one `Failed` — assert the expiry
         countdown text renders for the close-to-expiry cert and the failed cert is visually
         flagged.
      2. Click the failed certificate, respond to the resulting `SelectCertificateChain` command
         with a fixture `CertificateChainUpdated` whose Challenge stage has `is_stuck: true` and a
         `message` — assert the chain view renders all present stages and shows the Challenge as
         the stuck stage with its message visible.
      3. Reload/reconnect the pane against a fixture `CertManagerOverviewUpdated { installed: false, certificates: [] }` — assert the honest not-installed empty state renders.
      Keep the spec to these three scenarios only, no extra coverage.
- [x] T025 Run the full gate: `cargo build --workspace --all-targets`,
      `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui typecheck`,
      `pnpm -C ui lint`, `pnpm -C ui test`, and
      `pnpm -C ui exec playwright test ui/tests/e2e/cert-manager-pane.spec.ts`. Fix any failures
      before considering the feature done.

## Dependencies & Execution Order

- Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1) → Phase 4 (US2) → Phase 5 (US3) → Phase 6 (Polish/E2E).
- Phase 2 blocks all user stories (shared protocol types, backend scan/poll/command plumbing).
- US2 (Phase 4) depends on US1's `CertManagerPanel.tsx` existing (Phase 3) but not on US3.
- US3 (Phase 5) only needs Phase 2 + T017's conditional (already written in Phase 3) — it is test-only.
- T024's e2e spec depends on Phases 3-5 all being complete.

## Parallel Execution Examples

- Within Phase 2: T004 (round-trip tests) can run in parallel with T008-T009 (backend module) once
  T003/T005/T007 land, since they touch different files.
- Within Phase 3: T014 (store) and T016 (countdown formatter) are parallelizable; T017/T018 depend
  on both.
- T021 (resource descriptor) in Phase 4 is parallelizable with T019/T020 (panel selection/chain
  view) since it's a new, independent file, though T022 depends on all three.

## Implementation Strategy

**MVP = User Story 1 (Phase 1-3)**: detection + list + expiry/status is independently shippable
value per spec's own prioritization. US2 (chain drill-down) and US3 (empty-state polish, mostly
already free from US1's implementation) layer on top without touching US1's code paths.
