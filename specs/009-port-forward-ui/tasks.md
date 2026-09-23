---

description: "Task list for Port Forward Interface (009)"
---

# Tasks: Port Forward Interface

**Input**: Design documents from `specs/009-port-forward-ui/` (plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md)

**Tests**: Unit tests for port-picker debounce/conflict logic and origin-tagging merge logic, plus four Playwright e2e specs, are explicitly requested by the task brief. Included below.

**Organization**: Grouped by user story (US1–US4) per spec.md priorities (P1, P1, P2, P3).

## Path Conventions

Single project with `ui/` (React/TS frontend) and `src/` (Rust backend, untouched by this feature per FR-013/research.md). All paths below are relative to the feature worktree root.

---

## Phase 1: Setup

- [X] T001 Add `SECTION.portForwards = 'network/portforwards'` and `SECTION.services = 'network/services'` to the `SECTION` const in `ui/src/protocol/commands.ts`
- [X] T002 Add `REFRESH` map entries for `SECTION.portForwards` → `RefreshPortForwards` and `SECTION.services` → `RefreshServices` in `ui/src/protocol/commands.ts`
- [X] T003 Add a `selectCmd()` case for `kind === 'Service'` → `SelectService { namespace, name }` in `ui/src/protocol/commands.ts`

**Checkpoint**: protocol command plumbing for the new sections exists; nothing renders yet.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The Service descriptor and store state every user story's UI depends on (Pod already has ports; Service does not exist as a descriptor at all yet — research.md/data-model.md "prerequisite" decision).

**⚠️ CRITICAL**: Must complete before US1 (Service-forward path) and before any story's Inspector wiring lands cleanly.

- [X] T004 [P] Create minimal read-only `ResourceDescriptor<ServiceSummary, ServiceDetails>` in `ui/src/resources/services.tsx`, mirroring `ui/src/resources/nodes.tsx`'s structure: columns (name, type, cluster IP, port count), detail sections (metadata, ports, selector, related), `ref()`/`rowKey()` namespaced per `ui/src/resources/pods.tsx` conventions, `selectListEvent`/`selectDetail` wired to `ServicesUpdated`/`ServiceDetailsUpdated`
- [X] T005 Register the new `services` descriptor in `ALL_DESCRIPTORS` in `ui/src/resources/index.ts`
- [X] T006 Add a `PortAvailability` case to `handleSystem()` in `ui/src/store/handlers/system.ts` writing `integrations.portAvailability[port] = available` (confirmed absent in research.md; `tunnels`/`PortForwardTunnel` handling in `store/handlers/stream.ts` already exists, no change needed there)
- [X] T007 Add `portForwardOrigins: Record<string, { service?: string }>` to `StoreData` in `ui/src/store/index.ts` (ephemeral, not persisted, per data-model.md), plus an internal setter/action used by the start-flow to tag a tunnel id after a Service-launched `StartPortForward` succeeds and the matching `PortForwardsUpdated` row arrives (match on namespace/pod/local_port/remote_port, first unmatched row wins, per data-model.md's matching rule)

**Checkpoint**: Service descriptor exists and is navigable; store has the slices every story's components will read/write.

---

## Phase 3: User Story 1 - Start a port forward from a resource inspector (Priority: P1) 🎯 MVP

**Goal**: One-click "Forward" from a Pod or Service inspector, with a named confirmation step and a port picker with a sane default, resulting in a new row in the (not-yet-built) list once the backend confirms.

**Independent Test**: Open a Pod or Service inspector, invoke "Forward" on a port, confirm, observe a new entry with `Active` tunnel status and a bound local port (spec.md US1 Independent Test).

### Tests for User Story 1

- [X] T008 [P] [US1] Unit test for the debounced port-availability check helper (see T012) in `ui/src/components/panels/__tests__/PortForwardPortField.test.tsx`: asserts `CheckPortAvailability` is dispatched only after a 300ms debounce settles on edit (data-model.md "Port edits are debounced (300ms)"), not on every keystroke
- [X] T009 [P] [US1] Unit test for the origin-tagging merge logic (T007) in `ui/src/store/__tests__/portForwardOrigins.test.ts`: given a pending Service-launched target `{namespace, pod, localPort, remotePort, service}` and an incoming `PortForwardsUpdated` batch, asserts exactly the first matching, untagged `PortForward` row gets `portForwardOrigins[id] = {service}` and non-matching rows are untouched
- [X] T010 [US1] Playwright e2e spec `ui/tests/e2e/us1-port-forward-start.spec.ts`: covers spec.md US1 Acceptance Scenarios 1–5 (Pod-port forward confirm dialog naming namespace/pod/container/port; Service-port forward resolving a backing pod and naming it; confirm dispatches `StartPortForward` and row goes `Connecting`→`Active` via `PortForwardsUpdated`; cancel dispatches nothing; backend error shown verbatim and no misleading active row) — all new `data-testid`s prefixed `port-forward-` per contracts/protocol.md

### Implementation for User Story 1

- [X] T011 [US1] Create `PortForwardPortField` component in `ui/src/components/panels/PortForwardPortField.tsx`: controlled port `<input>`, 300ms-debounced `CheckPortAvailability` dispatch on edit, reads `integrations.portAvailability[port]` for conflict state, renders inline "port in use" warning (`data-testid="port-forward-port-conflict"`), used by the start dialog (US3's full conflict-detection acceptance criteria are exercised here structurally, though T008/T010 only assert the debounce mechanics needed for US1 to function — full US3 coverage lands in Phase 5)
- [X] T012 [US1] Create `PortForwardStartDialog` component in `ui/src/components/panels/PortForwardStartDialog.tsx`: props `{ target: ForwardTarget, open, onOpenChange }`; implements the `idle → confirming → submitting → closed|error` state machine from data-model.md; uses `ConfirmDialog` (`ui/src/components/ConfirmDialog.tsx`) naming namespace/pod/container?/port per FR-002; suggested port defaults to `remotePort` when `portAvailability[remotePort] !== false` else lets the operator edit via `PortForwardPortField`; on confirm dispatches `StartPortForward` via `request()`, shows backend error verbatim on failure (FR-009) and keeps dialog open; on success closes and (if `target.service` set) records the pending origin match for T007's tagging logic; `data-testid="port-forward-start-dialog"` / `"port-forward-start-confirm"`
- [X] T013 [US1] Create `PortForwardActionsPanel` component in `ui/src/components/panels/PortForwardActionsPanel.tsx`: the Pod/Service Inspector "Forward" tab body — for Pods, lists `PodDetails.ports` (`ContainerPort[]`) each with a "Forward" button (`data-testid="port-forward-action-{container}-{port}"`) plus a manual "Forward a different port" entry (edge case: no declared ports must not block); for Services, lists `ServiceDetails.ports` (`ServicePort[]`), on click first resolves a backing pod by sending `RefreshRelatedResources { section: network/services section key, kind: "Service", namespace, name }` (reusing the existing `RefreshRelatedResources` pattern per research.md), filters the reply to `kind === 'Pod'`, prefers `status_hint === 'Running'`, shows an explicit error state if none found/ready (edge case in spec.md); opens `PortForwardStartDialog` with the resolved `ForwardTarget`
- [X] T014 [US1] Wire a "Forward" `TabsTrigger`/`TabsContent` pair into `ui/src/components/Inspector.tsx`, conditional on `target.kind === 'Pod' || target.kind === 'Service'`, rendering `PortForwardActionsPanel`, following the existing tab-conditional pattern already used for other kind-specific tabs in that file

**Checkpoint**: An operator can start a forward from a Pod or Service inspector end-to-end; nothing renders the resulting list yet (that's US2's panel), but the store state (`lists[network/portforwards]`, `tunnels`) is already correctly populated via the pre-existing generic handlers.

---

## Phase 4: User Story 2 - Manage the live list of active forwards (Priority: P1)

**Goal**: A Port Forwards panel listing every forward for the current context with live status, Stop and Restart controls.

**Independent Test**: Seed the fake backend with 2-3 forwards in different states, open the panel, verify each row's fields, then exercise stop and restart per row — independent of US1's inspector-launch flow (spec.md US2 Independent Test).

### Tests for User Story 2

- [X] T015 [US2] Playwright e2e spec `ui/tests/e2e/us2-port-forward-manage.spec.ts`: covers spec.md US2 Acceptance Scenarios 1–6 (row fields incl. originating service; Stop confirm→`StopPortForward`→row stays non-`Closed` until backend confirms; Restart on `Error`/`Closed` reuses namespace/pod/port and runs through the same confirm+port-check flow; out-of-band `PortForwardTunnel` event updates a row live with no refresh; context switch clears prior-context rows; app-exit does not block/warn) — `data-testid`s prefixed `port-forward-`

### Implementation for User Story 2

- [X] T016 [US2] Create `PortForwardsPanel` component in `ui/src/components/panels/PortForwardsPanel.tsx`: dispatches `RefreshPortForwards` on mount (mirrors other tool panels' mount-refresh pattern per contracts/protocol.md); reads `lists[SECTION.portForwards]` and merges each `PortForward` with `tunnels[pf.id]` and `portForwardOrigins[pf.id]` into a `PortForwardRow` per data-model.md ("computed by merging... in the panel component, not stored merged"); renders `port-forward-list`/`port-forward-row` rows showing local port, namespace, pod, originating service badge when present, remote port, and live status (`tunnels[pf.id]?.status ?? pf.status`, satisfying FR-010/SC-006's never-anticipate rule); renders the dedicated empty state (FR-011, distinct from the generic tool-pane fallback) when the list is empty, explaining how to start a forward
- [X] T017 [US2] Add `PortForwardActionsPanel`-sibling row actions in `PortForwardsPanel.tsx` (or a small `PortForwardRowActions` sub-component in the same file): "Stop" button on `Active`/`Connecting` rows opening a `ConfirmDialog` naming the row's target, dispatching `StopPortForward { tunnel_id }` on confirm (`data-testid="port-forward-stop"`); "Restart" button on `Error`/`Closed` rows that opens `PortForwardStartDialog` seeded from the row's existing `namespace/pod/container/local_port/remote_port` (using `local_port` as the suggested port per data-model.md's restart-flow note), reusing the same component built in T012
- [X] T018 [US2] Add an `auto_start` toggle/badge per row in `PortForwardsPanel.tsx` (FR-012): a checkbox that dispatches `UpdatePortForward { tunnel_id, local_port, remote_port, auto_start }` re-sending the row's current `local_port`/`remote_port` unchanged (`data-testid="port-forward-auto-start"`)
- [X] T019 [US2] Wire `PortForwardsPanel` into `ui/src/components/shell/PaneHost.tsx`'s `ListBody`: render `<PortForwardsPanel />` when `section === SECTION.portForwards`, before the `!descriptor` fallback check — mirrors the existing `CustomResourcesSection` precedent (research.md's chosen entry-point design; phone/tablet's `FORWARDS_TAB` in `stack-tabs.ts` already routes here with no changes needed)

**Checkpoint**: US1 + US2 together are the MVP per plan.md — an operator can start, see, stop, and restart forwards end-to-end, laptop+ and phone/tablet.

---

## Phase 5: User Story 3 - Pick a local port with conflict detection (Priority: P2)

**Goal**: Full conflict-detection coverage on the port picker built structurally in US1 (T011): pre-filled checked suggestion, debounced re-check on edit, warning blocks confirm, clears on a free port, and a late backend bind failure still surfaces verbatim.

**Independent Test**: Occupy a local port externally, open the forward-start flow, enter that port, verify a conflict warning appears before confirm is allowed — independent of which resource type triggered the flow (spec.md US3 Independent Test).

### Tests for User Story 3

- [X] T020 [US3] Playwright e2e spec `ui/tests/e2e/us3-port-forward-port-picker.spec.ts`: covers spec.md US3 Acceptance Scenarios 1–4 (dialog loads with a pre-checked suggested port; editing to a fake-backend-flagged-unavailable port shows a blocking warning and disables confirm; editing to a free port clears the warning and re-enables confirm; confirming despite a race and a late backend bind failure shows the verbatim backend error per US1 Scenario 5) — `data-testid`s prefixed `port-forward-`

### Implementation for User Story 3

- [X] T021 [US3] In `PortForwardPortField.tsx` (T011), ensure the confirm action it feeds (`PortForwardStartDialog`'s confirm button) is disabled while `portAvailability[currentPort] === false` (FR-006's "surfacing a clear in-context conflict warning rather than allowing a blind attempt"); confirm re-enables once availability flips to `true` or the port is edited to an unchecked value pending its own debounced check
- [X] T022 [US3] Ensure `PortForwardStartDialog` (T012) pre-fills and immediately dispatches a `CheckPortAvailability` for the suggested port on open (not just on edit), satisfying US3 Scenario 1's "pre-filled... already been checked for availability"

**Checkpoint**: Full port-conflict-detection UX from spec.md US3 is in place; US1's dialog/field already had the mechanics, this phase closes the remaining acceptance-criteria gaps.

---

## Phase 6: User Story 4 - Copy the local address for use elsewhere (Priority: P3)

**Goal**: A copy-address action per row that copies `localhost:<local_port>` and confirms success, with a visible not-ready state for non-`Active` rows.

**Independent Test**: Open the list with an `Active` entry, use its copy-address action, verify the clipboard contains exactly `localhost:<local_port>` (spec.md US4 Independent Test).

### Tests for User Story 4

- [X] T023 [US4] Playwright e2e spec `ui/tests/e2e/us4-port-forward-copy-address.spec.ts`: covers spec.md US4 Acceptance Scenarios 1–2 (copy action on an `Active` row calls `navigator.clipboard.writeText` with exactly `localhost:<local_port>` and shows a confirmation toast; a `Connecting` row's copy action either is disabled or visibly indicates the tunnel is not yet ready) — `data-testid`s prefixed `port-forward-`

### Implementation for User Story 4

- [X] T024 [US4] Add a copy-address action per row in `PortForwardsPanel.tsx` (`data-testid="port-forward-copy-address"`): on `Active` rows, calls `navigator.clipboard.writeText(\`localhost:${local_port}\`)` and shows a `sonner` toast confirmation (mirrors `HelmReleaseActionsPanel.tsx`'s toast pattern per research.md); on non-`Active` rows, either disables the action or copies with a visible "tunnel not ready" indicator alongside it (FR-007/edge-case: state must be visible, wording is an implementation choice)

**Checkpoint**: All four user stories independently functional; panel is feature-complete per spec.md.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [X] T025 [P] Audit `tests/protocol_serde.rs` for existing round-trip coverage of `StartPortForward`/`StopPortForward`/`UpdatePortForward`/`CheckPortAvailability`/`PortForwardsUpdated`/`PortForwardTunnel`/`PortAvailability`/`RefreshServices`/`SelectService`/`ServicesUpdated`/`ServiceDetailsUpdated`; add tests only for any genuinely uncovered variant (research.md/plan.md: no protocol changes expected, so this should be a no-op or a small gap-fill, not new protocol code)
- [X] T026 [P] Update `docs/MVP.md`'s Port Forward Interface status row/note to reflect shipped status, matching the row-update pattern used by 006-helm-mutate's doc commit
- [X] T027 Run the full verification suite from `specs/009-port-forward-ui/quickstart.md` (`cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test -- --run`, and the four new Playwright specs) and fix any failures before considering the feature done

---

## Dependencies & Execution Order

- **Phase 1 (Setup)**: no dependencies.
- **Phase 2 (Foundational)**: depends on Phase 1 (needs `SECTION.services`/`SECTION.portForwards`). Blocks all user stories.
- **Phase 3 (US1)**: depends on Phase 2 (Service descriptor + store slices). Independently testable/shippable once done (list rendering not required for US1's own acceptance criteria, which only require the row to exist in store state).
- **Phase 4 (US2)**: depends on Phase 2; reuses `PortForwardStartDialog` from Phase 3 (T012) for its Restart action, so in practice lands after US1's T012, though its own Stop flow and panel rendering have no US1 dependency.
- **Phase 5 (US3)**: depends on Phase 3 (extends `PortForwardPortField`/`PortForwardStartDialog` built there).
- **Phase 6 (US4)**: depends on Phase 4 (adds an action to `PortForwardsPanel`).
- **Phase 7 (Polish)**: depends on all prior phases.

### Parallel Opportunities

- T004 (services descriptor) can proceed in parallel with T006/T007 (store handlers) — different files.
- T008/T009 (unit tests) can be written in parallel with each other and with T004.
- T025/T026 in Polish are independent of each other.

## Implementation Strategy

MVP = Phases 1–4 (Setup, Foundational, US1, US2) — matches plan.md's framing of US1+US2 as "the minimum useful slice." US3 and US4 are additive refinements layered on the same components without touching US1/US2's core flow.
