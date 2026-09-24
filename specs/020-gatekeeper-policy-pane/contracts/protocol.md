# Protocol Contract: Gatekeeper Policy Pane

This feature's UI ↔ backend contract, per Constitution Principle II (single versioned contract
owned by the backend, generated bindings for the UI). All types are defined once in
`src/protocol/v1/`; see `data-model.md` for full field definitions.

## Commands (UI → backend)

### `BackendCmd::RefreshGatekeeper`

- **Shape**: unit variant (no payload), identical shape to `RefreshFlux` / `RefreshHubble`.
- **Effect**: triggers `ensure_gatekeeper_poll` if the poll task isn't already running, and (per
  the existing Flux/Hubble pattern of "opening the pane also sends Refresh so it doesn't wait out
  the rest of the interval") the handler additionally does one immediate `scan_gatekeeper` +
  `GatekeeperOverviewUpdated` emit, decoupled from the 20s ticker.
- **Mutation classification**: read-only. MUST be added to `mutation.rs`'s non-mutating command
  list alongside `RefreshFlux`/`RefreshHubble` (FR-009).
- **Route**: `section_keys::integrations_gatekeeper()`.

## Events (backend → UI)

### `SystemEvent::GatekeeperOverviewUpdated(GatekeeperOverview)`

- **Emitted**: once per successful `poll_gatekeeper_overview` tick (every 20s) and once
  immediately in response to `RefreshGatekeeper`.
- **Payload**: `GatekeeperOverview` (see data-model.md) — always the full current snapshot, not a
  diff, matching `FluxOverviewUpdated`'s semantics.
- **Route**: `section_keys::integrations_gatekeeper()`, correlation `Correlation::None` (matches
  `poll_flux_overview`'s emit call).
- **Error path**: if `scan_gatekeeper` returns `Err` for a reason other than "CRDs not found"
  (e.g. transient API server error), the existing `events::notify_error` path is used (matches
  `poll_flux_overview`'s `Err(e)` arm) — this is distinct from `GatekeeperInstallState::PermissionDenied`,
  which is a *successful* scan that determined the operator lacks list/watch RBAC on the
  Gatekeeper CRDs specifically (a `Forbidden` API response while listing `Constraint`/
  `ConstraintTemplate`, caught and turned into `PermissionDenied` rather than bubbling as a
  generic scan failure, so User Story 3's third acceptance scenario gets its own honest state
  rather than a generic error toast).

## Frontend contract (`ui/src/store/index.ts`)

- New store slice `gatekeeper: { overview: GatekeeperOverview | null }`, initial value `overview:
  null` (mirrors `flux: { overview: null }`).
- New action `refreshGatekeeper()` sending `BackendCmd::RefreshGatekeeper` (mirrors
  `refreshFlux()`).
- Event reducer: on `SystemEvent::GatekeeperOverviewUpdated`, set `gatekeeper.overview` to the
  payload verbatim (mirrors the `FluxOverviewUpdated` case).

## UI navigation contract

- `GatekeeperPanel.tsx` calls `useStore((s) => s.openInspector)` and, for a clicked
  `GatekeeperViolation` row, calls:

  ```ts
  openInspector(
    { section: violation.section, kind: violation.kind, namespace: violation.namespace, name: violation.name },
    paneId,
    true,
  )
  ```

  identical call shape to `GitOpsWarningBanner.tsx`'s existing link-out. `violation.namespace`
  is always a concrete string here (never `null`) since Gatekeeper violation entries always name
  a namespaced or cluster-scoped resource by its actual namespace value (empty string for
  cluster-scoped, consistent with `FluxResourceSummary.namespace`'s existing convention — the
  frontend's `ObjectRef.namespace` accepts `string | null`, and the panel maps `""` to `null`
  before calling `openInspector`, matching how other panels already normalize empty-string
  cluster-scoped namespaces).

## Round-trip test coverage (Constitution Principle III)

- `src/protocol/v1/gatekeeper.rs` MUST include serde round-trip tests for `GatekeeperOverview`,
  `GatekeeperConstraintSummary`, `GatekeeperViolation`, and `GatekeeperInstallState` (all four
  variants), mirroring the existing round-trip test pattern in `src/protocol/v1/flux.rs` /
  `agent.rs`.
- `cmd.rs` and `event.rs` MUST include a round-trip test for the new `RefreshGatekeeper` command
  and `GatekeeperOverviewUpdated` event variants respectively, consistent with existing coverage
  for every other command/event variant (per Constitution Principle III, "every protocol command
  and event variant MUST have a serialization round-trip test").
- Generated TypeScript bindings (`ui/src/protocol/generated/Gatekeeper*.ts`) MUST be committed and
  checked for drift the same way existing generated files are (CI step already covers this
  generically — no new CI configuration needed).
