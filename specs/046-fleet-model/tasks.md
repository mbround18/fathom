---

description: "Task list for 046 fleet model"
---

# Tasks: A fleet model, because today there is only a context list

**Input**: Design documents from `/specs/046-fleet-model/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md

**Tests**: INCLUDED. Constitution Principle III is non-negotiable — every user story needs an E2E
test against the scripted fake backend, every protocol variant needs a round-trip test, UI decision
logic needs unit tests, and a behaviour change needs a test that fails without it.

**Organization**: grouped by user story so each is independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependencies on incomplete work)
- **[Story]**: US1 / US2 / US3, mapping to the spec's user stories
- Exact file paths in every description

## Path Conventions

Single repo: Rust backend under `src/`, React frontend under `ui/`. `pnpm` runs from `ui/`, never
the repo root. `CARGO_BUILD_JOBS=4` when other agents are active.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: nothing to scaffold — the crate, the package, the protocol modules and all three test
harnesses already exist. These two tasks only confirm the ground is where the plan says it is.

- [X] T001 Confirm the baseline gate is green before touching anything: run `cargo test -p fathom --test export_bindings` in the repo root and verify `git diff --stat ui/src/protocol/generated` is empty, so any later drift is this feature's
- [X] T002 [P] Confirm `~/.fathom/fleet.json` is unused today: grep `src/` for `fleet.json`, `fleet_file_path` and `load_fleet_model` in `src/backend/persistence.rs` and record that none exist

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the shared protocol types, the grammar, and the persistence they need. Every user
story depends on all of it.

**⚠️ CRITICAL**: no user story work can begin until this phase is complete.

### Protocol types (one contract — Principle II)

- [X] T003 Add `FleetMemberAvailability` (`Present` | `Absent`) and `SettingOrigin`-keyed `FleetMember { context: String (non-empty, the kubeconfig context name verbatim, never the API server URL), display_name: Option<String> (trimmed; empty-after-trim stored as None not "", max 128 chars), labels: BTreeMap<String, String>, origins: BTreeMap<String, SettingOrigin> (keyed `display_name` or `labels/<key>`), availability: FleetMemberAvailability }` to `src/protocol/v1/fleet.rs`, deriving `Serialize, Deserialize, TS`
- [X] T004 Add `FleetGroup { name: String (non-empty, trimmed, unique within the fleet, max 128 chars), selector: String (stored verbatim as the operator typed it), origin: SettingOrigin }` to `src/protocol/v1/fleet.rs`
- [X] T005 Add `FleetGapReason` with exactly four distinct variants — `NotPermitted { detail: String }`, `Unreachable { detail: String }`, `Absent`, `Pending` — and `FleetCoverageGap { context: String, reason: FleetGapReason }` to `src/protocol/v1/fleet.rs`. `detail` carries the API server's own reason verbatim. These variants MUST NOT collapse into one (FR-011)
- [X] T006 Add `FleetGroupRollup { group: String, members_total: u32, members_covered: u32, gaps: Vec<FleetCoverageGap>, node_ready: u32, node_total: u32, unhealthy_pods: u32 }` to `src/protocol/v1/fleet.rs`, with a doc comment stating the two invariants: `members_covered <= members_total`, and `gaps.len() == members_total - members_covered`. There must be no constructor or variant that yields the aggregates without the coverage fields (FR-010, SC-004)
- [X] T007 [P] Add `FleetMemberShare { display_name: Option<String>, labels: BTreeMap<String, String> }`, `SharedFleetDefinition { id: String, members: BTreeMap<String, FleetMemberShare>, groups: BTreeMap<String, String>, rev: u64, shared_by: String, updated_at: String (RFC 3339), removed: bool }` and `AdoptedFleetDefinition { definition: SharedFleetDefinition, adopted_at: String (RFC 3339), still_shared: bool }` to `src/protocol/v1/fleet.rs`, mirroring `ConnectionProfile` / `AdoptedConnectionProfile` in `src/protocol/v1/team.rs:137,158`. Document on `removed` and on `still_shared` that the local row is kept, never deleted (FR-014)
- [X] T008 Add the 11 commands from `contracts/protocol.md` to the `BackendCmdV1` enum in `src/protocol/v1/cmd.rs`: `RefreshFleetModel`, `SetFleetMemberDisplayName { context, display_name }`, `SetFleetMemberLabels { context, labels }`, `ForgetFleetMember { context }`, `SaveFleetGroup { name, selector }`, `DeleteFleetGroup { name }`, `RequestFleetGroupRollup { group }`, `RefreshSharedFleetDefinitions`, `PublishFleetDefinition { id }`, `AdoptFleetDefinition { id }`, `RemoveFleetDefinition { id }`
- [X] T009 Add the 6 events from `contracts/protocol.md` beside `FleetSummaryUpdated` in `src/protocol/v1/event.rs:538`: `FleetModelUpdated { members, groups }`, `FleetModelRejected { reason, field }`, `FleetModelLoadFailed { path, reason }`, `FleetGroupRollupUpdated(FleetGroupRollup)`, `SharedFleetDefinitionsUpdated { definitions, adopted }`, `FleetStoreUnavailable { reason }`
- [X] T010 Add a serialization round-trip test for every new command variant in the `#[cfg(test)]` block at the tail of `src/protocol/v1/cmd.rs`, following the existing `roundtrip(BackendCmdV1::SetTeamStorePath { .. })` form at line 1961 (FR-019)
- [X] T011 Add a serialization round-trip test for every new event variant and every new type in `src/protocol/v1/event.rs` and `src/protocol/v1/fleet.rs` test modules, including one `FleetGapReason` case per variant so a collapsed enum fails the build (FR-019)
- [X] T012 Add one hosted-policy arm per new command in `src/protocol/v1/mutation.rs` per `contracts/protocol.md`: the member and group commands allowed only when `context == home`, the sharing commands allowed under the existing role gate. Add a test asserting `StartFleetPolling`, `StopFleetPolling` and `RequestFederationSnapshot` still return `HostedDenial::Fleet` and a non-`home` `SetContext` still returns `HostedDenial::ContextSwitch` — no existing denial relaxed (FR-018)
- [X] T013 Regenerate the TypeScript bindings: `cargo test -p fathom --test export_bindings`, then commit `ui/src/protocol/generated/`. `scripts/ci.sh:75-83` fails the build if this is skipped

### Grammar and matching (backend only — Principle II)

- [X] T014 Create `src/backend/fleet_model.rs` with the label validator: a key is an optional DNS-subdomain prefix (1–253 chars, lowercase alphanumeric plus `-` and `.`) followed by `/`, then a name of 1–63 characters, alphanumeric at both ends, interior may also use `-`, `_`, `.`; a value is empty or that same 1–63-character name shape. The error MUST name the failing input and the rule it broke, verbatim (FR-003)
- [X] T015 Add the selector parser to `src/backend/fleet_model.rs`: parse the `kubectl -l` subset — comma-separated requirements, `k=v`, `k==v`, `k!=v`, `k in (a,b)`, `k notin (a,b)`, `k` (Exists), `!k` (DoesNotExist) — into `Vec<kube::core::Expression>` and thence `kube::core::Selector`. The parse error MUST carry the offending substring so FR-006's "verbatim" has content. `kube-core` has no `FromStr`; see research §1
- [X] T016 Add group membership to `src/backend/fleet_model.rs` using `kube::core::SelectorExt::matches(&BTreeMap<String, String>)` against a member's labels — do not re-implement set semantics. A member appears in every group whose selector matches it (FR-004)
- [X] T017 [P] Add a table test in `src/backend/fleet_model.rs` covering every accepted form in T015 round-tripping through `Display` back to a string `ListParams::labels()` accepts, and rejecting `env in prod`, `=prod`, `env=`, a 64-character label name and a key with two slashes — each with the offending substring asserted in the message
- [X] T018 [P] Add a table test in `src/backend/fleet_model.rs` for the label validator: valid keys with and without a prefix, valid empty value, and each invalid shape with its rule named

### Persistence (Principle VI)

- [X] T019 Add `fleet_file_path()`, `fleet_file_version() -> u32` (currently `1`), `load_fleet_model()` and `save_fleet_model()` to `src/backend/persistence.rs`, following the `TeamConfig` discipline at `persistence.rs:669-700`. File shape `{ schema: u32, members, groups, adopted }` per `data-model.md`. An unreadable or invalid file yields in-memory defaults and leaves the file **byte-for-byte untouched**, reporting the verbatim reason (FR-007). Unknown keys at a known schema are ignored
- [X] T020 Add a test in `src/backend/persistence.rs` that writes a truncated and a non-JSON `fleet.json`, asserts defaults are returned, asserts the verbatim reason is reported, and asserts the file's **bytes** before and after are identical — not merely that no error surfaced (SC-003)
- [X] T021 [P] Add a test asserting a serialized `fleet.json` contains no credential-shaped value, reusing `scan_fields_for_secrets` from `src/protocol/v1/team.rs` if it applies (FR-020)
- [X] T022 Add a test asserting that saving the fleet model writes only `fleet.json` — `cluster-settings.json` and `federation.json` are unmodified (FR-008)

### Command plumbing

- [X] T023 Create `src/backend/cmd/fleet.rs` with `handle(cmd, state, sender) -> anyhow::Result<Option<BackendCmd>>` in the family-chain form `src/backend/cmd/team.rs:21-78` uses, claiming the 11 new commands and returning the rest. Register it in the chain in `src/backend/cmd.rs`
- [X] T024 Implement `RefreshFleetModel` in `src/backend/cmd/fleet.rs`: load the file, reconcile each member's `availability` against the contexts in the current kubeconfig, compute group membership, emit `FleetModelUpdated`. On a load failure emit `FleetModelLoadFailed`. Start no poller, client or watch (FR-015)

**Checkpoint**: protocol, grammar and persistence are in place; the three user stories can proceed in parallel.

---

## Phase 3: User Story 1 — Name and label the clusters I actually have (Priority: P1) 🎯 MVP

**Goal**: an operator with ARN-shaped context names can identify every cluster from its card, and
the names and labels survive a restart.

**Independent Test**: with three contexts in the kubeconfig, set a display name and two labels on
one, reload, and confirm both are shown beside the still-copyable context string; enter an invalid
label and confirm the backend's own message appears and nothing is persisted.

- [X] T025 [US1] Add the E2E spec `ui/tests/e2e/fleet-model-identity.spec.ts` against the scripted fake backend (`emit`/`open`/`sent` from `./helpers`, builders from `../fixtures/builders`): display name and context string both visible on one card, context string copies verbatim, invalid label rejected with the backend's verbatim message and no `SetFleetMemberLabels` persisted, both values present after reload. Write it red, before T026–T028
- [X] T026 [US1] Implement `SetFleetMemberDisplayName` in `src/backend/cmd/fleet.rs`: trim, empty → `None`, mark the `display_name` origin local, persist, emit `FleetModelUpdated`
- [X] T027 [US1] Implement `SetFleetMemberLabels` and `ForgetFleetMember` in `src/backend/cmd/fleet.rs`: validate every key and value through `fleet_model` **before writing anything**; on failure emit `FleetModelRejected { reason, field }` with the verbatim message and persist nothing. `ForgetFleetMember` is the only path that removes metadata and works whether the member is `Present` or `Absent` (FR-003, FR-009)
- [X] T028 [US1] Extend `ui/src/components/fleet/FleetContextCard.tsx` to show the display name beside the context string at line 57 — both, never one instead of the other — keep the context string copyable verbatim, and render the member's labels. Render `Absent` members as unavailable with their metadata intact (FR-002, FR-009)
- [X] T029 [P] [US1] Add `ui/src/components/fleet/FleetMemberEditor.tsx`: display name and label entry, surfacing `FleetModelRejected.reason` verbatim against the named field. No client-side grammar — the backend owns it (Principle II)

**Checkpoint**: US1 is independently shippable and satisfies SC-001 and SC-003.

---

## Phase 4: User Story 2 — Group clusters by selector and see each group's rollup (Priority: P2)

**Goal**: one selector expression defines a group; a newly labelled cluster joins every matching
group with no further action; and every aggregate states its own coverage.

**Independent Test**: label three contexts, define two overlapping groups, confirm each group lists
the right members and that a rollup over a group with one denied member reports `n of m`, names
that member, and calls it *not permitted* — distinctly from absent and from unreachable.

- [X] T030 [US2] Add the E2E spec `ui/tests/e2e/fleet-model-groups.spec.ts`: a selector-defined group lists its members with no per-cluster filing step; labelling a new cluster joins it to every matching group with no further action; a group matching nothing says so and is neither hidden nor removed; ungrouped members are reachable through an explicit listing; a rollup with one denied member shows `n of m`, names the member and labels it not permitted. Write it red, before T031–T035
- [X] T031 [US2] Implement `SaveFleetGroup` and `DeleteFleetGroup` in `src/backend/cmd/fleet.rs`: parse the selector through `fleet_model` **first**; on a parse error emit `FleetModelRejected` carrying the error verbatim and create no group (FR-006). Upsert by name; store the expression verbatim as typed
- [X] T032 [US2] Implement `RequestFleetGroupRollup` in `src/backend/cmd/fleet.rs`: aggregate the `FleetContextSummary` values already held for the matching members, emit `FleetGroupRollupUpdated`. Issue no new cluster requests (FR-015). Map each uncovered member to a `FleetCoverageGap`: a 403 → `NotPermitted { detail }` with the API server's own reason, a connection failure → `Unreachable { detail }`, a context missing from the kubeconfig → `Absent`, a member not yet polled → `Pending`
- [X] T033 [US2] Add a backend test asserting the two rollup invariants hold for a mixed fleet — `members_covered <= members_total` and `gaps.len() == members_total - members_covered` — and that a denied member never reduces to the same reason as an absent one (FR-010, FR-011)
- [X] T034 [US2] Add `ui/src/resources/fleet.ts`: the filter/sort/group decision logic — which members a label filter or group shows, the "showing n of m" count, and the ungrouped partition. Pure functions, no I/O
- [X] T035 [US2] Extend `ui/src/components/fleet/FleetDashboardView.tsx` (today it maps `session.contexts` straight to cards at line 60): group sections, a label/group filter stating how many of how many members it shows, the per-group coverage line, and an explicit ungrouped listing. Every aggregate renders with its coverage; a rollup with gaps names them and their reasons (FR-010, FR-016, FR-017)
- [X] T036 [P] [US2] Add `ui/src/components/fleet/FleetGroupEditor.tsx`: group name and selector entry, surfacing the parse error verbatim and creating no group on failure
- [X] T037 [P] [US2] Add `ui/tests/unit/fleet-filter.test.ts`: the decision logic from T034 over a 100-member fixture, asserting the "n of m" count and that filtering responds within the 100 ms budget (SC-007)

**Checkpoint**: US2 satisfies SC-002, SC-004 and SC-007.

---

## Phase 5: User Story 3 — Share one definition of "prod" with the team (Priority: P3)

**Goal**: labels and groups publish to and adopt from the 014 store, with origin shown, local
values winning, and an adopted copy that is never deleted.

**Independent Test**: publish labels and two groups from one profile, adopt from a second with its
own local override, and confirm the groups match identically while the local label wins and is
marked local; then make the store unreachable and confirm the adopted copy survives, marked no
longer following.

- [ ] T038 [US3] Add the E2E spec `ui/tests/e2e/fleet-model-sharing.spec.ts`: publish-then-adopt reproduces the same group names and memberships; a locally overridden label wins, is marked local, and the overridden shared value stays visible; store unreachable and upstream removal both leave the adopted copy in place, marked no longer following. Write it red, before T039–T042
- [ ] T039 [US3] Extend `src/backend/team_store.rs` with `fleet-definitions.json` — `{ rev: u64, definitions: BTreeMap<String, SharedFleetDefinition> }` — and `read_fleet_definitions`, `publish_fleet_definition`, `remove_fleet_definition`, mirroring `read_profiles` / `publish_profile` / `remove_profile`. Every write optimistic-concurrency-checked against the caller's last seen `rev` (`StoreError::Conflict`, never a silent overwrite) and atomic via temp-file + rename
- [ ] T040 [US3] Implement `RefreshSharedFleetDefinitions`, `PublishFleetDefinition`, `AdoptFleetDefinition` and `RemoveFleetDefinition` in `src/backend/cmd/fleet.rs`. `PublishFleetDefinition` behind `gate(state, sender, FathomRole::Editor)` as `PublishConnectionProfile` is. `AdoptFleetDefinition` MUST NOT overwrite any value whose origin is local. Emit `FleetStoreUnavailable { reason }` with `StoreError`'s own message on failure (FR-012, FR-013)
- [ ] T041 [US3] Implement the never-delete rule in `src/backend/cmd/fleet.rs`: an unreachable store or an upstream `removed: true` sets `still_shared: false` on the adopted local copy and nothing more. Add a test asserting no code path deletes an adopted copy (FR-014)
- [ ] T042 [US3] Add a backend test asserting origin resolution: a local value wins, is reported as local, and the shared value it overrode remains readable; `SharedFleetDefinitionsUpdated` includes `still_shared: false` rows rather than dropping them (FR-013, FR-014)
- [ ] T043 [US3] Extend `ui/src/components/fleet/FleetDashboardView.tsx` and `FleetMemberEditor.tsx` to show each value's origin, mark local overrides, keep the overridden shared value visible, and offer publish/adopt/remove. Render `FleetStoreUnavailable` without implying the adopted copies are gone
- [ ] T052 Close the Principle II hole found during implementation: `FleetModelUpdated` carried members and groups but **no mapping between them**, so `ui/src/resources/fleet.ts` grew a second selector evaluator in TypeScript. Add `groups: Vec<String>` to `FleetMember` and a way for a group to report an unparseable stored selector (so the UI can tell "matches nothing" from "cannot be read" without parsing), populate both in `src/backend/cmd/fleet.rs` from `fleet_model::groups_matching`, regenerate the bindings, and **delete `matchesSelector`/`selectorReadable` and every caller** so the grammar lives in the backend only. Also narrow `FleetMemberEditor`'s rejection check to `labels/` fields, so a `field: "selector"` refusal no longer surfaces inside every open member editor.
- [ ] T051 Surface `FleetModelLoadFailed` (FR-007, US1 acceptance scenario 4): add the `SystemEvent` case and store field in `ui/src/store/handlers/system.ts` / `ui/src/store/handlers/fleet.ts`, and render the path and the verbatim reason in `ui/src/components/fleet/FleetDashboardView.tsx` **without implying the operator's file was discarded** — the backend leaves the bytes untouched and falls back to defaults, so the wording must say the file could not be read, not that the fleet is empty. Add it to the US3 E2E spec. Gap found during implementation: no task in Phase 3 covered this event.

**Checkpoint**: US3 satisfies SC-006.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [ ] T044 Add `ui/tests/hosted/fleet-model.spec.ts`: display name, labels and sharing work for `home`; a command naming any other context is denied; the four pre-existing denials still deny (FR-018)
- [ ] T045 [P] Verify SC-005 — run `./scripts/measure-idle-cpu.sh` and compare against the pre-046 figure; the feature must add no measurable idle CPU and no additional API server requests
- [ ] T046 [P] Confirm the fleet grid still reflows at every width per Principle IV: `grid-cols-1 sm:grid-cols-2 lg:grid-cols-3` with the new label row and group headers, no fixed-width surface
- [ ] T047 [P] Add the UI note that fleet labels are operator-authored metadata, not Kubernetes objects, and are never read from or written to any cluster object (spec Assumptions, Principle I)
- [ ] T048 Run the full gate: `./scripts/ci.sh` green, including the bindings-drift step
- [ ] T049 Capture the promo screenshots for the new fleet screens (`cd ui && pnpm promo`) — the final step of the batch
- [ ] T050 Run `./scripts/hygiene.sh` and confirm it exits 0 with nothing dangling; flip the 046 row in `docs/MVP.md` to ✅

---

## Dependencies & Execution Order

- **Phase 1** (T001–T002) → **Phase 2** (T003–T024) blocks everything else.
- Within Phase 2: T003–T007 (types) → T008–T009 (commands/events) → T010–T012 (round-trips, hosted
  policy) → T013 (bindings). T014–T018 (grammar) and T019–T022 (persistence) are independent of the
  protocol tasks and of each other. T023–T024 need the protocol and the persistence.
- **Phases 3, 4 and 5 are independent of one another** once Phase 2 is done, except that T035 and
  T043 both edit `FleetDashboardView.tsx` and T029 and T043 both edit `FleetMemberEditor.tsx` — US3's
  UI task must land after US2's, or the two must be done by the same agent.
- **Phase 6** needs all three stories.

## Parallel Example: Phase 2

```
T003 → T004 → T005 → T006   (same file, sequential)
T007 [P]                    (same file — serialize with the above)
T014 → T015 → T016          (new file, independent of the protocol tasks)
T019                        (persistence.rs, independent of both)
```

Three agents: one on the protocol chain (T003–T013), one on grammar (T014–T018), one on persistence
(T019–T022). They converge on T023–T024.

## Implementation Strategy

**MVP** is Phase 2 + Phase 3 (US1): a fleet whose members have names and labels that persist. That
alone fixes the "which cluster is this ARN" problem the review found, and it is shippable without
groups or sharing.

**Increment 2** is US2, which is what turns the list into a fleet and produces the selector
expression 047 consumes.

**Increment 3** is US3, which makes the model a team artifact.

**Parallelisation**: after Phase 2, US1/US2/US3 split cleanly across agents by file, with the two
shared UI files noted above as the only ordering constraint. Keep `./scripts/ci.sh` as a distinct
fast gate rather than bundling verification at the tail of one long run.

## Notes

- `pnpm` runs from `ui/`, never the repo root. Playwright sizes its own workers
  (`ui/tests/support/workers.ts`); pass no `--workers`. Two Playwright suites at once run this box
  out of memory. Use `CARGO_BUILD_JOBS=4` when other agents are active.
- Land each coherent piece as its own signed commit once its gate is green; do not batch to the end.
