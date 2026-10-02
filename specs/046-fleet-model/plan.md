# Implementation Plan: A fleet model, because today there is only a context list

**Branch**: `046-fleet-model` | **Date**: 2026-10-02 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/046-fleet-model/spec.md`

## Summary

Today the fleet is a list. `FleetDashboardView` maps `session.contexts` to cards in kubeconfig
order (`ui/src/components/fleet/FleetDashboardView.tsx:60`) and `FleetContextSummary` carries no
identity beyond the context string (`src/protocol/v1/fleet.rs:45`), so an operator with twelve ARN-
shaped contexts has no way to say which cluster is which, no way to address a subset, and no
aggregate that states what it covers.

This feature gives the fleet a model: a member keyed on context name carrying an operator-set
display name and a label map; a group that is a **named label selector** over those labels rather
than a folder tree; a rollup that cannot express a sum without stating its coverage and naming
every member it could not read; persistence in its own version-stamped `~/.fathom/fleet.json`; and
sharing through the store `014-teams-organizations` already configures, reusing that feature's
revision, attribution and never-delete semantics.

Technically: `kube::core::Selector`/`Expression` as the selector representation (for `matches()`
and for `Display` back to the string `ListParams::labels()` takes — which hands 047 its scope
expression for free), with the string parser written here because `kube-core` has none; validation
and all persistence in the Rust backend; new commands and events in the existing protocol modules
with generated TypeScript; no pollers, clients or watches added.

## Technical Context

**Language/Version**: Rust (edition 2024) backend; TypeScript 5 / React 19 frontend.

**Primary Dependencies**: `kube` 2.0.1 (`runtime`, `derive`, `ws`), `k8s-openapi` 0.26.1 (v1_34),
`serde`, `ts-rs` 12 (protocol bindings), `crossbeam-channel` (command/event plumbing), Tauri v2,
shadcn/ui.

**Storage**: a new `~/.fathom/fleet.json` (`{ schema: u32, members, groups, adopted }`) read and
written by `src/backend/persistence.rs`; plus one new file, `fleet-definitions.json`, in the
operator-controlled 014 shared store (`src/backend/team_store.rs`). No database. No new file
anywhere else, and nothing added to `cluster-settings.json` or `federation.json` (FR-008).

**Testing**: `cargo test` (round-trips, selector parser, label validator, persistence fallback,
hosted policy); Vitest for UI decision logic; Playwright E2E against the scripted fake backend
(`ui/tests/e2e/`), plus the hosted project; `scripts/ci.sh` as the gate, including its
bindings-drift step.

**Target Platform**: Linux desktop (MVP is Linux-only: unsigned deb/rpm/AppImage), plus the hosted
web build.

**Project Type**: desktop application with a Rust backend and a React frontend in one repo.

**Performance Goals**: filtering a 100-member fleet responds to input in under 100 ms (SC-007);
zero additional idle CPU and zero additional API server requests over today's behaviour (SC-005).

**Constraints**: no background work introduced — no pollers, no clients, no watches (FR-015); a
corrupt metadata file must cost the operator nothing and must not be modified (FR-007); no
credentials in the persisted file (FR-020); no existing hosted denial relaxed (FR-018).

**Scale/Scope**: fleets of up to ~100 members; 20 functional requirements, 3 user stories, 11 new
commands, 6 new events, 6 new protocol types; roughly 2 new backend modules, 1 extended protocol
module, and the existing fleet view and card in the UI.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Initial gate (before Phase 0) — PASS

| Principle | Assessment |
|---|---|
| **I. Operator-first, explicit mutation** | PASS. Groups are label selectors — the thing an operator already types into `kubectl -l` — not a folder tree this feature invents and then has to explain. Every write is an explicit operator action; nothing is inferred or auto-filed. No wizard, and no Kubernetes object is hidden: labels here are deliberately *not* cluster objects (spec Assumptions), which the UI must say. |
| **II. One contract** | PASS. Label grammar, selector parsing, matching and all persistence live in the Rust backend; the UI holds no second copy of the grammar. New commands/events are defined in Rust, TypeScript is generated, and `scripts/ci.sh:75-83` fails the build on drift. |
| **III. Tested at the boundaries (NON-NEGOTIABLE)** | PASS with obligations recorded: one E2E spec per user story against the fake backend, a round-trip test per new protocol variant (FR-019), unit tests for the UI's filter/group decision logic, and a red-first test for the coverage invariant. See `quickstart.md`. |
| **IV. Density scales with width** | PASS. The feature reuses the existing fleet grid (`grid-cols-1 sm:grid-cols-2 lg:grid-cols-3`) and adds a label row and group header to it; no fixed-width surface. |
| **V. Fast and quiet** | PASS, and this is the one with teeth: FR-015 forbids any background work, so the feature's cost is a file read at launch. Filtering is client-side over a bounded in-memory list with a 100 ms budget (SC-007). `RequestFleetGroupRollup` aggregates summaries already held and issues no cluster requests of its own. |
| **VI. User data is sacred** | PASS. `schema: u32` with additive-only change; an unreadable file yields defaults and is left byte-for-byte untouched (asserted on bytes, not on absence of error); no credentials in the file; adoption never deletes a local copy and never overwrites a local value. |

No violations. Nothing to record in Complexity Tracking.

### Post-design re-check (after Phase 1) — PASS

The design added two things worth re-examining against the constitution.

1. **A hand-written selector parser** (research §1). This is new surface we own rather than
   borrow, which Principle II's "one contract" invites scrutiny of. It stays compliant because it
   lives once, in the backend, and produces the library's own `kube::core::Expression` — so
   matching is upstream's tested code and the only thing we own is the string → expression step
   that `kube-core` genuinely does not provide (no `FromStr`, no `parse`, verified in
   `kube-core-2.0.1/src/labels.rs`). The alternatives were worse by the constitution's own
   lights: an opaque string validated only by an API server cannot satisfy FR-006's reject-at-
   entry, and a `matchLabels`/`matchExpressions` form is the wizard Principle I rules out.

2. **A second file in the 014 shared store** (research §4). Principle VI governs it: the file is
   written atomically (temp + rename) and revision-checked, as `team_store.rs` already does for
   `connection-profiles.json`, so a crash or a concurrent teammate cannot corrupt or silently
   overwrite it. Adoption is additive and local-wins, and `removed` keeps the row rather than
   deleting it — which is `ConnectionProfile.removed`'s existing contract, not a new one.

The coverage types tightened under Principle I rather than loosened: `FleetGroupRollup` has no
variant that can state a sum without `members_covered`/`members_total`, and `FleetGapReason` keeps
`NotPermitted`, `Unreachable` and `Absent` distinct. That is deliberate — it makes the
`integrationVisible` mistake (`ui/src/resources/nav.ts:64-69`, where a *denied* CRD scan renders as
"not installed") unrepresentable here.

Still no violations. Complexity Tracking stays empty.

## Project Structure

### Documentation (this feature)

```text
specs/046-fleet-model/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── protocol.md      # Phase 1 output — commands, events, hosted policy, UI contract
└── tasks.md             # Phase 2 output (/speckit-tasks — not created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── fleet.rs              # extended: FleetMember, FleetGroup, FleetGroupRollup,
│   │                         #   FleetCoverageGap, FleetGapReason, SharedFleetDefinition,
│   │                         #   AdoptedFleetDefinition (+ round-trip tests)
│   ├── cmd.rs                # extended: 11 new BackendCmdV1 variants (+ round-trip tests)
│   ├── event.rs              # extended: 6 new BackendEventV1 variants (+ round-trip tests)
│   └── mutation.rs           # extended: hosted policy arm per new command; existing
│                             #   HostedDenial::Fleet / ContextSwitch arms unchanged
├── backend/
│   ├── fleet_model.rs        # new: label validation, selector parse/match, group membership,
│   │                         #   rollup assembly with coverage
│   ├── cmd/fleet.rs          # new: the command handler, in the existing handle() chain
│   ├── persistence.rs        # extended: load_fleet_model / save_fleet_model, fleet_file_path,
│   │                         #   fleet_file_version — TeamConfig's fallback discipline
│   └── team_store.rs         # extended: read/publish/remove fleet definitions in
│                             #   fleet-definitions.json, rev-checked, atomic
tests/
└── export_bindings.rs        # unchanged; regenerating it is a task, not a change

ui/
├── src/
│   ├── protocol/generated/   # regenerated, committed, drift-gated by scripts/ci.sh
│   ├── components/fleet/
│   │   ├── FleetDashboardView.tsx   # extended: group sections, filter, coverage line,
│   │   │                            #   ungrouped listing
│   │   ├── FleetContextCard.tsx     # extended: display name beside the context string, labels
│   │   ├── FleetGroupEditor.tsx     # new: name + selector, verbatim parse error
│   │   └── FleetMemberEditor.tsx    # new: display name + labels, verbatim validation error
│   └── resources/fleet.ts           # new: filter/sort/group decision logic (unit-tested)
└── tests/
    ├── e2e/fleet-model-identity.spec.ts   # US1
    ├── e2e/fleet-model-groups.spec.ts     # US2
    ├── e2e/fleet-model-sharing.spec.ts    # US3
    ├── hosted/fleet-model.spec.ts         # FR-018
    └── unit/fleet-filter.test.ts          # SC-007 and the "n of m" count
```

**Structure Decision**: the existing single-repo layout, unchanged — a Rust backend under `src/`
with the protocol defined once in `src/protocol/v1/`, and the React frontend under `ui/` consuming
generated bindings. This feature adds two backend modules and extends four protocol modules; it
introduces no new project, crate or package, and it adds no new persistence mechanism beyond one
file in `~/.fathom/` and one file in the store 014 already owns.

## Complexity Tracking

> No Constitution Check violations in either gate. Nothing to justify.
