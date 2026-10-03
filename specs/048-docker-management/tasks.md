# Tasks: Docker and container-runtime management

**Input**: Design documents from `/specs/048-docker-management/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/protocol.md](./contracts/protocol.md),
[quickstart.md](./quickstart.md)

**Tests**: REQUIRED, not optional. Constitution Principle III is non-negotiable: every user story
needs E2E tests against the scripted fake backend, every protocol variant needs a round-trip test,
and every behaviour change needs a test that fails without it.

**Organization**: grouped by user story so each is independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable — different files, no dependency on an incomplete task
- **[Story]**: US1–US7 for user-story phases only

## Resource rules for every task in this file

One image (`busybox`, ~4 MB). Never prune, never remove an image, never stop a container the test
did not create — this daemon hosts `thunderforge-app` and `slam-usa`. No kind cluster is created for
this feature. `CARGO_BUILD_JOBS=4` or lower; prefer `cargo check` while iterating. Playwright sizes
its own workers; do not pass `--workers`.

---

## Phase 1: Setup

- [ ] T001 Create the backend module tree `src/backend/container_runtime/` with `mod.rs` and empty modules `probe.rs`, `inventory.rs`, `grouping.rs`, `adoption.rs`, `preview.rs`, `blast_radius.rs`, `mutate.rs`, `stats.rs`, `logs.rs`, `session.rs`, `compose.rs`, `cluster_link.rs`; register the tree in `src/backend/mod.rs`
- [X] T002 Make the runner abstraction reusable: widen the visibility of `Runner`, `SystemRunner` and `RunError` in `src/backend/kind.rs` so `container_runtime` can use them, leaving `detect_prerequisites_with` behaviour unchanged (its existing tests must still pass untouched)
- [X] T003 Add a streaming child-process capability alongside `Runner` in `src/backend/kind.rs` (or a sibling trait in `src/backend/container_runtime/mod.rs`) for long-running invocations — log follow and image build — with a mock implementation for tests; one-shot `Runner::run` stays as it is
- [ ] T004 [P] Create `ui/src/features/containers/` and `ui/tests/e2e/` placeholders for this feature, and confirm `ui/tests/support/` is where the fake backend handlers will go

---

## Phase 2: Foundational (blocking — every user story depends on this)

### Protocol types (one file, so sequential within the file)

- [X] T005 Create `src/protocol/v1/container_runtime.rs` with `RuntimePresence` and `RuntimePresenceState` — exactly four variants `Absent | Stopped | Unreachable | Ready`, no fifth and no catch-all; `runtime: Option<KindRuntimeKind>` is `Some` whenever state is not `Absent`; `endpoint: Option<String>` is `None` only when `Absent`; `detail: Option<String>` carries the runtime's verbatim message when `Unreachable`
- [X] T006 Add `Container`, `PublishedPort`, `ContainerState` and `ContainerHealth` to `src/protocol/v1/container_runtime.rs`. `ContainerState` includes `Restarting` and `Unknown(String)` — an unrecognised value is preserved, never collapsed. `health: Option<ContainerHealth>` is `None` when the runtime reports none and is never defaulted to healthy. `id` is the full identity, not the short form. `PublishedPort.host_port` is `Option<u16>`, `None` for exposed-but-unpublished
- [X] T007 Add `Project`, `ProjectGroup` (with a distinct `Unowned` variant so no real project can collide with the no-project bucket), `DefinitionFileState` (`Present | Missing | Unreadable(String) | Undeclared`) and `AdoptionState` (`Adopted | NotAdopted | ImplicitViaManagedCluster`) to `src/protocol/v1/container_runtime.rs`; `running <= members` and both are carried
- [X] T008 Add `RemovableObject`, `ObjectClass`, `ObjectReference` and `ReferenceKind` to `src/protocol/v1/container_runtime.rs`. `tagged: Option<bool>` is never `None` for an image; `size_bytes: Option<u64>` is `None` when the runtime does not report it, never `0`
- [X] T009 Add `OperationPreview`, `RemovalLine`, `ExcludedObject`, `ExclusionReason::UnadoptedProject(String)` and `ClusterBreak { cluster, connected }` to `src/protocol/v1/container_runtime.rs`. `command: Vec<String>` is the literal argv, program first — not a prose rendering
- [X] T010 Add `OperationOutcome`, `ObjectOutcome`, `OutcomeStatus` and `Discrepancy` to `src/protocol/v1/container_runtime.rs`; `overall` is `Succeeded` only when every per-object entry succeeded, and `error: Option<String>` is the runtime's stderr verbatim and unwrapped
- [X] T011 [P] Add `UsageSample` to `src/protocol/v1/container_runtime.rs` with numeric `cpu_percent: Option<f32>`, `memory_bytes: Option<u64>`, `memory_limit_bytes: Option<u64>` — the runtime's display strings are parsed in the backend and never carried
- [X] T012 Add `OperationRequest` and its payload specs (`ContainerSpec`, `ImageBuildSpec`, `NetworkSpec`, `VolumeSpec`, `OperationScope`, `LifecycleAction`, `ProjectAction`, `PruneFilter`) to `src/protocol/v1/container_runtime.rs`. **No field of type "extra arguments", "raw command" or equivalent on any of them** — such a field makes FR-019 unkeepable by construction
- [X] T013 Register `container_runtime` in `src/protocol/v1/mod.rs` with the re-exports the rest of the protocol needs

### Commands, events, hosted policy

- [X] T014 Add the command variants from `contracts/protocol.md` to `src/protocol/v1/cmd.rs`: `RefreshContainerRuntimePresence`, `RefreshContainerInventory`, `LoadProjectDefinition`, `AdoptContainerProject`, `ReleaseContainerProject`, `PreviewContainerOperation`, `IssueContainerOperation`, the log/session/usage commands
- [X] T015 Add the event variants from `contracts/protocol.md` to `src/protocol/v1/event.rs`, including `ContainerInventoryUpdated { .., as_of }` so a stale list can be marked rather than blanked, `ContainerAdoptionUpdated { .., fell_back }`, and `ContainerOperationRefused { reason: OperationRefusal }` with `NotAdopted(String) | PreviewStale | ConfirmationMismatch | PresenceLost`
- [X] T016 Add `HostedDenial::ContainerRuntime` to `src/protocol/v1/mutation.rs` with the doc comment stating the reason — a server has no local runtime, and exposing the host's daemon to signed-in users would grant privileges they do not have — and add the `hosted_policy` arms denying every new command with it. Do **not** reuse `HostedDenial::Kind`, whose documented meaning is local kind provisioning

### Contract tests (Principle III)

- [X] T017 [P] Round-trip tests for every new command variant in the `src/protocol/v1/cmd.rs` test module, following the existing `roundtrip(...)` pattern
- [X] T018 [P] Round-trip tests for every new event variant in the `src/protocol/v1/event.rs` test module
- [X] T019 [P] Test in `src/protocol/v1/mutation.rs` asserting every new command returns `HostedPolicy::Denied(HostedDenial::ContainerRuntime)` — asserted, not assumed
- [X] T020 [P] Test asserting no `OperationRequest` payload type carries a free-text argument field, so FR-019 cannot erode in a later change
- [X] T021 Run `cargo test -p fathom --test export_bindings` and commit the generated TypeScript in `ui/src/protocol/generated/`; confirm `git diff --stat ui/src/protocol/generated` is empty afterwards

### Adoption — the gate everything else passes through

- [X] T022 Implement the adoption store in `src/backend/container_runtime/adoption.rs`: read and write `~/.fathom/container_adoption.json` via `src/backend/persistence.rs::fathom_config_dir()`, keyed by project name with `adopted_at` RFC 3339
- [X] T023 Tests for the store: a grant persists and reloads; an unreadable or malformed record yields the **empty** set and leaves the file on disk unmodified; the fallback is reported through `fell_back: true` and never silently treated as "everything adopted"
- [X] T024 Implement the single mutation gate in `src/backend/container_runtime/adoption.rs`. It takes the command (not a boolean), so a new mutating command that forgets the gate does not compile. Non-mutating reads, log following and usage sampling pass regardless of adoption; interactive sessions do **not** (FR-013)
- [X] T025 Tests for the gate: every mutating command refused for an unadopted project with `OperationRefusal::NotAdopted`; a session refused on an unadopted container; logs and usage allowed; granting project `A` affects no object whose project is `B`
- [X] T026 Implement implicit adoption for Fathom-managed clusters in `src/backend/container_runtime/cluster_link.rs` + `adoption.rs`: a container whose cluster is one Fathom manages resolves to `ImplicitViaManagedCluster`, is actionable without a grant, and cannot be released
- [X] T027 Tests for implicit adoption, including that cluster-node containers — which carry no project label and so fall in the no-project bucket — are not left unactionable, and that `ReleaseContainerProject` refuses for an implicit grant

### Presence and dispatch

- [X] T028 Implement `src/backend/container_runtime/probe.rs` mapping the existing `kind::detect_prerequisites` result onto `RuntimePresence`, including the `Unreachable` case where the binary exists and the daemon answers with a permission error; reuse the existing `tokio::time::timeout` + `spawn_blocking` wrapper rather than adding a second probe
- [X] T029 Tests for presence against a mock runner covering all four states, asserting that none of them produces a "no containers" message and that `Unreachable` carries the runtime's own text and names the endpoint
- [X] T030 Create `src/backend/cmd/container_runtime.rs` with the dispatch skeleton for every new command and wire it into `src/backend/cmd.rs`; each arm routes through the adoption gate where applicable
- [ ] T031 [P] Add fake-backend handlers for every new command and event to `ui/tests/support/`, scriptable enough to reach the awkward acceptance scenarios: unreachable socket, 500 containers, corrupt definition file, stale preview
- [ ] T032 [P] Add the UI store slice for runtime state in `ui/src/store/` and the sidebar entry in `ui/src/layout/`, rendered only when presence is not `Absent` (FR-002) — the entry's absence is a presence decision, not a feature flag

**Checkpoint**: protocol is defined once and generated, adoption gates every mutation and fails
closed, presence is honest in four states, and nothing is rendered yet.

---

## Phase 3: User Story 1 — See what is actually running (P1) 🎯 MVP

**Goal**: containers grouped by project, with honest runtime states.

**Independent test**: with containers from three projects plus two standalone containers running,
four groups appear with the right members and each container's state, image, ports and uptime match
what the runtime reports.

### Tests first

- [ ] T033 [P] [US1] E2E spec `ui/tests/e2e/us1-container-inventory.spec.ts` covering all six acceptance scenarios: section absent when `Absent`; "installed and stopped" naming the runtime; three project groups with member and running counts; the no-project bucket named as such; the runtime going away mid-scene marking the list stale rather than blanking it; permission failure naming the socket with its remediation. Poll request ids with `requestIdOf(page, '<Type>')` after the click — never `nextRequestId()` before it
- [ ] T034 [P] [US1] Unit tests in `src/backend/container_runtime/inventory.rs` against a mock runner with canned output, including a label set whose `com.docker.compose.depends_on` value contains both commas and equals signs — the parser must not misattribute the container's project (research D2)

### Implementation

- [ ] T035 [US1] Implement `src/backend/container_runtime/inventory.rs`: container listing via `--format '{{json .}}'`, with the project and cluster labels read through targeted `{{.Label "<key>"}}` projections and a batched `inspect --format '{{.Id}} {{json .Config.Labels}}'` only when the full label set is needed. Never split the flat `Labels` string
- [ ] T036 [US1] Implement `src/backend/container_runtime/grouping.rs`: group by declared project, put label-less containers in `ProjectGroup::Unowned`, and compute `members` and `running` per group
- [ ] T037 [US1] Implement project definition discovery in `src/backend/container_runtime/compose.rs`: resolve `definition_file` from the project's config-files label and set `definition_state`. Read only — never write, move or delete the operator's file
- [ ] T038 [US1] Implement `ContainersScene.tsx` and `ContainerGroupList.tsx` in `ui/src/features/containers/`: virtualized rows, group headings carrying name + member count + running count, the adoption badge per group, mobile-first density with no horizontal scroll, WCAG 2.2 AA
- [ ] T039 [US1] Render the four presence states distinctly, including stale-marking on `ContainerInventoryUnavailable` using `as_of`, and the permission case naming the socket with its remediation

**Checkpoint**: US1 is independently usable — the operator can see what is running and whose it is.

---

## Phase 4: User Story 2 — Act on a container or a project (P2)

**Goal**: lifecycle operations behind a preview that is literally what will run.

**Independent test**: an unadopted project offers no action and says why; after adoption, stopping a
container previews that exact operation and object, and the row changes only when the runtime
reports.

### Tests first

- [ ] T040 [P] [US2] E2E spec `ui/tests/e2e/us2-container-lifecycle.spec.ts` covering all eight acceptance scenarios, including the unadopted project explaining itself and offering adoption, adoption surviving a restart, release returning to read-only, per-container outcomes on a project operation, a partial failure not reporting overall success, and a container removed between preview and confirmation surfacing "no such container" verbatim
- [ ] T041 [P] [US2] **The argv invariant test** in `src/backend/container_runtime/mutate.rs`: issuing an operation executes argv byte-identical to what the preview carried. Without this test the preview is decoration — it is the most important test in the feature
- [ ] T042 [P] [US2] Unit tests for per-object outcome reporting: a four-member project where one member fails reports four outcomes and an overall status that is not `Succeeded`

### Implementation

- [ ] T043 [US2] Implement lifecycle previews in `src/backend/container_runtime/preview.rs`: build `OperationPreview` with the literal argv, enumerate every target, and register the preview under a `preview_id` so issuing can re-derive it
- [ ] T044 [US2] Implement `src/backend/container_runtime/mutate.rs`: issue the previewed argv, refuse with `PreviewStale` when the inventory moved in a way that changes the target set, collect per-object outcomes, and surface the runtime's stderr verbatim
- [ ] T045 [US2] Implement `AdoptionControl.tsx` in `ui/src/features/containers/`: adopt, release, and the explicit "not adopted" explanation with adoption offered — a control is never silently absent (FR-017)
- [ ] T046 [US2] Implement `OperationPreview.tsx` in `ui/src/features/containers/`: show the literal command, the enumerated targets, and require confirmation; no optimistic state change anywhere in the path
- [ ] T047 [US2] Wire project-scoped lifecycle: one preview naming every member, one confirmation, per-member outcomes reported separately

**Checkpoint**: the daily-workflow win is in place — a stuck service restarts without a terminal.

---

## Phase 5: User Story 3 — Reclaim space without breaking another project (P3)

**Goal**: the panes that answer "what is safe to delete", and a destructive path that cannot reach
an unadopted project.

**Independent test**: with dangling images, a tagged image referenced by a stopped container, and an
unused volume present, each object's size and references are correct; a prune selection previews
exactly what disappears, names every project touched, and requires a typed confirmation.

### Tests first

- [ ] T048 [P] [US3] E2E spec `ui/tests/e2e/us3-reclaim-space.spec.ts` covering all six acceptance scenarios, including that a prune cannot be issued while any object is summarised as a count alone, and that a cancelled removal issues nothing
- [ ] T049 [P] [US3] Unit tests in `src/backend/container_runtime/blast_radius.rs`: the radius names projects other than the selected one; an object belonging to an unadopted project is **excluded** with `ExclusionReason::UnadoptedProject`, not merely warned about; an object referenced by both an adopted and an unadopted project is excluded
- [ ] T050 [P] [US3] Unit tests for post-verification: a re-read inventory differing from the preview produces a `Discrepancy` that is reported rather than swallowed

### Implementation

- [ ] T051 [US3] Implement removable-object inventory in `src/backend/container_runtime/inventory.rs`: images, volumes and networks with sizes and `referenced_by`, distinguishing tagged from dangling images, and keeping an unreported size as `None` rather than `0`
- [ ] T052 [US3] Implement `src/backend/container_runtime/blast_radius.rs`: compute the removal set from Fathom's own inventory — the runtime offers no dry-run (research D3) — name every project reached, and exclude objects belonging to unadopted projects
- [ ] T053 [US3] Implement prune as a reviewed *selection* resolving to targeted removals by identity, never a broad prune handed to the runtime; an operation whose scope cannot be enumerated is not offered at all (FR-027)
- [ ] T054 [US3] Extend `mutate.rs` with the destructive guard: typed confirmation checked against the object being destroyed, then post-verification against a re-read inventory with any discrepancy reported
- [ ] T055 [US3] Implement `ObjectPanes.tsx` in `ui/src/features/containers/` for images, volumes and networks with size, tagged/dangling, references and in-use state
- [ ] T056 [US3] Extend `OperationPreview.tsx` for destructive operations: the full removal list with nothing reduced to a count, total space reclaimed, the blast-radius project names, the excluded objects with their reason, and the typed-confirmation field

**Checkpoint**: the operator can reclaim disk without being able to damage another project.

---

## Phase 6: User Story 4 — Containers and clusters are one world (P4)

- [ ] T057 [P] [US4] E2E spec `ui/tests/e2e/us4-cluster-crosslink.spec.ts` covering all five acceptance scenarios, including the break warning naming the cluster and stating separately that the operator is connected to it, and a look-alike container with no cluster identity having no cluster claimed
- [ ] T058 [P] [US4] Unit tests in `src/backend/container_runtime/cluster_link.rs` from mock runner output carrying the cluster label — no kind cluster is created for these tests
- [ ] T059 [US4] Implement `cluster_link.rs`: resolve cluster identity from the label only, never from naming patterns, in both directions
- [ ] T060 [US4] Surface the backing containers in the existing cluster view, linking back to the Docker section
- [ ] T061 [US4] Implement `ClusterLinkBadge.tsx` and extend the preview with `ClusterBreak`: warn that the cluster will break, name it, and state separately when the operator is currently connected
- [ ] T062 [US4] Reflect an externally stopped node container in both the container state and the cluster's availability — neither may claim the cluster is healthy

---

## Phase 7: User Story 5 — Logs and a shell (P5)

- [ ] T063 [P] [US5] E2E spec `ui/tests/e2e/us5-logs-and-session.spec.ts` covering all five acceptance scenarios, including a stopped container showing retained output with no stream left open, a container with no usable shell surfacing the runtime's own failure, and navigating away leaving nothing streaming
- [ ] T064 [P] [US5] Unit tests for the stopped-container log path and for session teardown leaving no child process
- [ ] T065 [US5] Implement `src/backend/container_runtime/logs.rs` as an adapter onto the existing `src/backend/log_streams.rs`, reusing its bounding, batching and explicit truncation marking; allowed regardless of adoption (FR-035)
- [ ] T066 [US5] Implement `src/backend/container_runtime/session.rs` as an adapter onto the existing `src/backend/node_shell.rs` PTY machinery; gated on adoption (FR-013), closing the view ends the session with no process left behind
- [ ] T067 [US5] Wire both into the scene, reusing the existing log and terminal components rather than new ones

---

## Phase 8: User Story 6 — See what is eating the machine (P6)

- [ ] T068 [P] [US6] E2E spec `ui/tests/e2e/us6-container-usage.spec.ts` covering all five acceptance scenarios, including an unavailable figure rendering as unavailable rather than zero, and the bounded history's cap being visible
- [ ] T069 [P] [US6] Unit tests for parsing the runtime's formatted figures ("24.56MiB / 31.25GiB", "0.00%") into numbers, and for per-project aggregation stating how many members' figures were unavailable rather than summing them as zero
- [ ] T070 [US6] Implement `src/backend/container_runtime/stats.rs`: discrete `stats --no-stream --format '{{json .}}'` ticks scoped to explicit visible ids, minimum 2 s interval, reusing the bounded-history approach from 036; **no long-lived sampler** (research D4)
- [ ] T071 [US6] Implement `UsagePanel.tsx` with per-container and per-project figures, and stop sampling on navigation away or scene close (FR-045)

---

## Phase 9: User Story 7 — Create, run and build (P7)

- [ ] T072 [P] [US7] E2E spec `ui/tests/e2e/us7-create-and-build.spec.ts` covering all six acceptance scenarios, including build output streaming with the builder's own failure text, project-down treated as destructive, a missing definition file disabling project operations without the file being rewritten, and the absence of any free-text command field being stated plainly rather than implied impossible
- [ ] T073 [P] [US7] Unit tests for each creation preview's argv, and for the project-down path being classified destructive
- [ ] T074 [US7] Implement form-driven creation previews and issuance for container, network and volume in `preview.rs` + `mutate.rs`, over the documented fields only
- [ ] T075 [US7] Implement image build using the streaming runner from T003, with live output and the builder's own failure text surfaced verbatim
- [ ] T076 [US7] Implement project up/down/restart in `compose.rs` + `mutate.rs`, naming the definition file in the preview, treating down as destructive, and never writing the file
- [ ] T077 [US7] Implement the creation forms in `ui/src/features/containers/`, with the explicit statement that a case no form covers belongs in a terminal (FR-034) — and no free-text command field

---

## Phase 10: Polish and cross-cutting

- [ ] T078 [P] Verify SC-012: 500 containers, operator input handled under 100 ms, measured against the fake backend's 500-container script
- [ ] T079 [P] Verify SC-011 and FR-045 with `./scripts/measure-idle-cpu.sh --attach <pid>`: idle figure indistinguishable from the pre-feature baseline and under 2% of one core, **and** zero `docker`/`podman` child processes spawned while the scene is closed
- [ ] T080 [P] Accessibility pass on the new scene: WCAG 2.2 AA, keyboard reachability of every action and of the typed confirmation, no horizontal scroll at narrow widths
- [ ] T081 [P] Verify SC-013: a corrupt definition file leaves that file byte-identical on disk, and a lost adoption record leaves nothing actionable rather than everything
- [ ] T082 Run the full gate: `./scripts/ci.sh > /tmp/ci-048.log 2>&1; echo "ci_exit=$?"` — capture the exit code explicitly rather than piping to `tail`, which has already produced one false green here. Bindings-drift step included
- [ ] T083 Capture the promo screenshots for the new Docker screens (`cd ui && pnpm promo`) — the final step of the batch
- [ ] T084 Run `./scripts/hygiene.sh`, confirm it exits 0 with nothing dangling, remove the `fathom048` fixtures by name (never by prune), and flip the 048 row in `docs/MVP.md`

---

## Dependencies

```text
Phase 1 Setup
   └─▶ Phase 2 Foundational  (protocol · adoption gate · presence · dispatch)
          ├─▶ Phase 3 US1 (P1)  ──▶ MVP
          ├─▶ Phase 4 US2 (P2)   [needs US1's inventory for targets]
          ├─▶ Phase 5 US3 (P3)   [needs US2's preview/issue path]
          ├─▶ Phase 6 US4 (P4)   [needs US1; extends US2/US3 previews]
          ├─▶ Phase 7 US5 (P5)   [needs US1 + the adoption gate]
          ├─▶ Phase 8 US6 (P6)   [needs US1]
          └─▶ Phase 9 US7 (P7)   [needs US2's preview/issue path]
                 └─▶ Phase 10 Polish
```

US5, US6 and US4 depend only on US1 plus the foundation, so they can proceed alongside US2/US3 once
the preview path exists. US3 and US7 both extend `preview.rs` and `mutate.rs`, so they should not be
implemented concurrently by two agents.

## Parallel opportunities

- **Phase 2**: T017–T020 (contract tests) in parallel; T011 alongside T005–T010 is safe only if the
  file is split by hand, otherwise keep the `container_runtime.rs` tasks sequential
- **Phase 2 tail**: T031 (fake backend) and T032 (store + sidebar) in parallel with the backend work
- **Every story phase**: its test tasks are `[P]` with each other and are written before the
  implementation tasks in that phase
- **Phase 10**: T078–T081 in parallel; T082 → T083 → T084 strictly in order

## Implementation strategy

**MVP is Phase 1 + Phase 2 + Phase 3 (US1).** That alone answers "what is running and whose is
it" on a shared daemon, which is the question the operator asks most and the one no amount of
`docker ps` output makes legible.

**Then US2**, which is the reason the feature was asked for.

**Land each coherent piece as its own signed commit** rather than batching to the end — protocol
types, then the adoption gate, then each story phase. Every commit GPG-signed; verify
`commit.gpgsign` and `user.signingkey` are active, including that no local repo override has
appeared, before committing.

**Two things not to defer**: the argv invariant test (T041), because without it every preview in
the feature is decoration; and the blast-radius exclusion (T052), because the shared daemon is the
reason the feature needs a guard at all.
