# Implementation Plan: Resource Editing, Node Actions & Custom Resource Browsing

**Branch**: `003-yaml-node-crd-parity` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-yaml-node-crd-parity/spec.md`

## Summary

Add three explicitly-confirmed mutating capabilities plus one new read-only discovery capability,
all following patterns the codebase already has proven for exactly this shape of problem:

- **Edit YAML + apply** (US1): the Inspector's YAML tab currently renders read-only YAML
  client-side from `raw_json` via a hand-rolled `jsonToYaml()` (`ui/src/resources/yaml.ts`) — there
  is no YAML parser on either side today. Editing stays as raw text in the UI; the backend does the
  only YAML *parsing* needed, via a new `serde_yaml` dependency, converting the operator's edited
  text into the target resource's typed struct and calling `Api::replace()`. Optimistic-concurrency
  conflict detection falls out of `replace()` for free: the edited YAML must carry the
  `resourceVersion` it was opened at, and Kubernetes itself rejects a stale one with 409 — no
  bespoke locking. The diff is computed by re-fetching the live object and diffing text with a new,
  minimal frontend diff library (none exists in `ui/package.json` today).
- **Node actions** (US2): cordon/uncordon are a `Patch::Merge` of `spec.unschedulable` on
  `Api<Node>` (mirrors the existing `Api<Node>::all()` read pattern in `src/backend/cmd/cluster.rs`
  fetch_node_summaries/select_node). Taint is the same merge-patch shape against
  `spec.taints`. Drain uses the real Kubernetes Eviction subresource
  (`Api<Pod>::evict()`, confirmed present in kube 2.0.1's default `client` feature — no new
  dependency or flag) so PodDisruptionBudget enforcement is the server's job, not ours; a background
  task emits incremental progress events, reusing the exact `StreamEvent`/`StreamClosed` streaming
  convention `002-pod-logs-terminal` established for logs/exec.
- **Generic CRD browsing** (US3): `kube::discovery` + `kube_core::dynamic::DynamicObject` (both in
  kube 2.0.1's default `client` feature, confirmed — no new dependency) enumerate installed CRDs via
  `apiextensions.k8s.io` and list/get their instances generically, replacing what Flux/Longhorn
  currently do by hand-coding one CRD each. Instances render through the same generic
  Details/YAML/Events Inspector tabs already used for built-in kinds — no parallel UI.

All five new mutating command kinds (`ApplyResourceYaml`, `CordonNode`, `UncordonNode`,
`DrainNode`, `TaintNode`) are classified in `mutation.rs` as `Namespaced`/`ClusterScoped` (never
`ReadOnly`), which is what routes them through Fathom's existing write-action confirmation UI
(`ConfirmDialog`/`ActionDialogHost`) — the same mechanism already gating delete/scale/restart.

## Technical Context

**Language/Version**: Rust (edition 2024, backend) / TypeScript strict (frontend)

**Primary Dependencies**: `kube` 2.0.1 (`client` feature, already default-enabled — provides
`Api::evict`, `Api::replace`, `Api::patch`, `kube::discovery`, `kube_core::dynamic::DynamicObject`;
no new Rust dependency or feature flag needed), new `serde_yaml` (backend, YAML text ↔ typed
object for the apply path — the crate is on crates.io as `serde_yaml = "0.9"`, upstream-flagged
deprecated with no functional replacement shipped yet; pinned as a stable, widely-used release per
constitution's "dependencies SHOULD be stable" — see research.md R4 for the alternatives considered
and why this is still the right call today); new frontend diff-viewer dependency (`ui/`, see
research.md R5 — none exists in `ui/package.json` today)

**Storage**: N/A — resource-edit sessions, drain progress, and CRD catalog data are all live,
in-memory state; nothing persisted to `~/.fathom/`

**Testing**: `cargo test` (backend unit tests for YAML→object parsing, node-patch construction,
drain eviction-loop, CRD discovery/listing, following `flux.rs`/`hubble.rs` conventions),
Playwright E2E against the scripted fake backend (`ui/tests/live`/`ui/tests/e2e` conventions)
driving the UI by injecting `StreamEvent`s/`SystemEvent`s and asserting emitted `BackendCmdV1`s,
Vitest unit tests for store reducers and the diff computation

**Target Platform**: Tauri v2 desktop app — Ubuntu 26.04, Windows 11, macOS 26 (Tier 1, per
constitution)

**Project Type**: Desktop app (Rust backend + Tauri/React frontend), single repo

**Performance Goals**: Per constitution Principle V — input handled under 100ms even with a drain
or CRD-instance list streaming updates; CRD instance lists must be virtualized like every other
resource list, not held as unbounded history

**Constraints**: Every new mutating command MUST be explicitly confirmed before send (spec FR-021);
apply MUST reject identity changes and stale-version applies before any cluster write (spec
FR-004/FR-005); drain MUST use the real Eviction API, not client-side PDB math (spec FR-009/FR-010);
CRD browsing MUST stay read-only in this feature (spec FR-020)

**Scale/Scope**: 1 new backend module (`src/backend/apply.rs`), node actions added to the existing
node command handling in `src/backend/cmd/cluster.rs` + a new `src/backend/nodes.rs` drain helper
(promoting node logic that today lives inline in `cluster.rs`... see research.md R2), 1 new backend
module (`src/backend/crds.rs`) for generic discovery/listing, ~6 new `BackendCmdV1` variants
(`ApplyResourceYaml`, `CordonNode`, `UncordonNode`, `DrainNode`, `CancelDrainNode`, `TaintNode`,
`RefreshCrds`, `SelectCrdKind`, `RefreshCrdInstances`, `SelectCrdInstance` — see contracts/), ~4 new
`StreamEvent`/`SystemEvent` variants, 2-3 new frontend components (editable YAML view + diff panel,
generic CRD nav/list, node-action controls), 1 new frontend dependency

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation** — PASS. All five new mutating actions
  (apply/cordon/uncordon/drain/taint) are classified `Namespaced`/`ClusterScoped` in
  `mutation.rs`, which is exactly what makes them route through the existing
  `ConfirmDialog`/`ActionDialogHost` write-confirmation flow (spec FR-003/FR-008/FR-009/FR-013/
  FR-021) — no new confirmation mechanism invented. No optimistic UI: the YAML tab shows "applying"
  until the backend confirms, node state only flips after the backend reports success, drain
  progress is reported incrementally from the backend, never assumed client-side. Backend errors
  (validation failure, 409 conflict, PDB block, permission denied) reach the operator verbatim
  (spec FR-007/FR-010/FR-019), matching the constitution's explicit requirement.
- **II. One Contract, Backend Owns the Cluster** — PASS. All YAML parsing/apply, node
  patch/evict/taint, and CRD discovery/listing happens only in the Rust backend
  (`src/backend/apply.rs`, `src/backend/nodes.rs`, `src/backend/crds.rs`); the frontend only sends
  commands and renders events via generated ts-rs bindings. No direct cluster access from the UI,
  no new `~/.fathom/` reads/writes.
- **III. Tested at the Boundaries** — PASS (planned). Every new `BackendCmdV1`/`StreamEvent`
  variant gets a serialization round-trip test (extending `tests/protocol_serde.rs`'s exhaustive
  match helpers — the exact gap that silently broke for the exec variants in
  `002-pod-logs-terminal` and was only caught by running `cargo clippy --all-targets`, so this plan
  explicitly calls out running the full `--all-targets` invocation, not `--lib --bins`, before
  considering any phase done). New E2E scenarios drive apply/cordon/drain/taint/CRD-browsing
  against the fake backend fixture. Unit tests cover YAML→object parsing edge cases (identity-field
  changes, malformed YAML), the eviction-loop's handling of 429/PDB responses, and CRD discovery's
  empty/permission-denied states.
- **IV. Density Scales With Width** — PASS. The editable YAML view and diff panel live inside the
  existing Inspector YAML tab slot (no new layout slot). Node actions are added as controls on the
  existing node details view. Generic CRD browsing adds one new top-level nav section using the
  same resource-list/inspector layout pattern every other section already uses — no new responsive
  behavior to design.
- **V. Fast and Quiet** — PASS (design requirement carried into Phase 1). CRD instance lists reuse
  the existing virtualized list component. Drain progress events are one small event per pod
  evicted, not a poll loop — bounded by the node's actual pod count. The diff computation runs once
  per "review diff" click, not on every keystroke.
- **VI. User Data Is Sacred** — PASS. Nothing here touches `~/.fathom/`. No secrets are persisted;
  a Secret's YAML, if opened for editing, is handled exactly as read-only Secret viewing already is
  today (no new masking behavior needed or removed — out of scope to change existing Secret display
  behavior, this feature only adds edit/apply to whatever the resource's current read view already
  shows).

No violations. Complexity Tracking table is not needed.

**Post-Phase-1 re-check** (after research.md/data-model.md/contracts/quickstart.md were written):
still PASS on all six principles. The `serde_yaml` deprecation-status call (research.md R4) and the
choice to promote node-drain logic out of the inline `cluster.rs` handlers into a dedicated
`nodes.rs` module (research.md R2) are internal structuring decisions, not protocol or trust-
boundary changes, and don't introduce new violations.

## Project Structure

### Documentation (this feature)

```text
specs/003-yaml-node-crd-parity/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/            # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── cmd.rs            # + ApplyResourceYaml, CordonNode, UncordonNode, DrainNode,
│   │                        CancelDrainNode, TaintNode, RefreshCrds, SelectCrdKind,
│   │                        RefreshCrdInstances, SelectCrdInstance
│   ├── event.rs          # + ResourceApplied, NodeActionFinished, DrainProgress (reuses
│   │                        StreamClosed for drain-stream end); CrdCatalogUpdated,
│   │                        CrdInstancesUpdated in system.rs
│   └── mutation.rs        # classify all 6 new mutating commands (Namespaced/ClusterScoped);
│                            CRD-read commands classified ReadOnly
├── backend/
│   ├── apply.rs             # NEW: YAML text -> typed object, resourceVersion conflict
│   │                          check, Api::replace() dispatch by kind
│   ├── nodes.rs              # NEW: promotes node-summary/detail helpers out of cluster.rs
│   │                          (kept call-compatible) + cordon/uncordon/taint patches +
│   │                          drain eviction-loop task (mirrors ExecSessionState/
│   │                          LogStreamState singleton-task pattern in state.rs)
│   ├── crds.rs                # NEW: kube::discovery scan for apiextensions.k8s.io CRDs,
│   │                          Api<DynamicObject> list/get for instances
│   └── cmd/
│       ├── cluster.rs          # node command dispatch delegates to backend::nodes
│       └── workloads.rs        # (unchanged; apply/CRD dispatch lives in new cmd modules
│                                  or is added here per tasks.md's call)

ui/
├── src/
│   ├── protocol/generated/    # ts-rs regenerated: new cmd/event TS types
│   ├── resources/
│   │   └── yaml.ts              # unchanged (still used for read-only render fallback)
│   ├── store/
│   │   ├── index.ts               # + ResourceEditSession, DrainProgressState, CrdCatalog
│   │   │                            in StoreData
│   │   └── handlers/
│   │       ├── stream.ts           # + DrainProgress/ResourceApplied handling
│   │       └── system.ts           # + CrdCatalogUpdated/CrdInstancesUpdated handling
│   └── components/
│       ├── Inspector.tsx           # YAML tab gains an edit affordance + diff review step
│       ├── panels/
│       │   ├── YamlEditTab.tsx      # NEW: editable textarea + "Review diff" + confirm
│       │   ├── YamlDiffView.tsx     # NEW: renders the diff using the new diff dependency
│       │   └── NodeActionsPanel.tsx # NEW: cordon/uncordon/drain/taint controls + drain
│       │                             progress, wired through ActionDialogHost
│       └── nav/
│           └── CustomResourcesSection.tsx # NEW: CRD catalog nav + generic list, reusing
│                                             the existing resource-list/Inspector pattern
├── package.json                  # + one diff-viewer dependency (research.md R5)
└── tests/
    ├── e2e/                       # + Playwright scenarios: apply+diff+conflict, cordon/
    │                                drain/taint, CRD browse/empty/permission-denied
    └── unit/                       # + diff computation, drain progress reducer tests
```

**Structure Decision**: Single-repo desktop app, existing `src/` (Rust backend) + `ui/`
(Tauri/React frontend) split, following `002-pod-logs-terminal`'s precedent exactly. Three new
backend modules beside the existing `pods.rs`/`flux.rs`/`longhorn.rs` pattern; four new frontend
components beside the existing `panels/` directory — no new top-level directories.

## Complexity Tracking

*No violations — table not needed.*
