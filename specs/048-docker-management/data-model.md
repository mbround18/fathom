# Phase 1 Data Model: Docker and container-runtime management

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-10-02

Types live once in `src/protocol/v1/container_runtime.rs` with `ts-rs` derives; TypeScript is
generated into `ui/src/protocol/generated/`. Field-level constraints below are normative — quote
them in the implementing task rather than deciding them at implementation time.

---

## RuntimePresence

What the operator is told before any inventory exists. Derived from the existing
`KindPrerequisites` probe rather than a second detector (FR-001).

| Field | Type | Constraint |
|---|---|---|
| `state` | `RuntimePresenceState` | required; one of the four values below |
| `runtime` | `Option<KindRuntimeKind>` | `Some` whenever `state` is anything but `Absent` |
| `endpoint` | `Option<String>` | the daemon this process is talking to; `None` only when `Absent` |
| `detail` | `Option<String>` | the runtime's own message, verbatim, when `Unreachable` |

`RuntimePresenceState` — exactly four variants, no fifth and no catch-all (FR-003):

- `Absent` — nothing installed. The scene does not appear (FR-002).
- `Stopped` — installed, not running.
- `Unreachable` — running, but this account cannot reach it. `detail` carries why, `endpoint`
  names the socket.
- `Ready` — running and reachable.

**Invariant**: no code path may render "no containers" for any state other than `Ready` with an
empty inventory (SC-001).

---

## Container

| Field | Type | Constraint |
|---|---|---|
| `id` | `String` | full runtime identity, not the short form; every operation targets this (FR-011) |
| `name` | `String` | display only; never used to target an operation |
| `image` | `String` | as reported |
| `state` | `ContainerState` | `Created`, `Running`, `Paused`, `Restarting`, `Exited`, `Dead`, `Removing`, `Unknown(String)` — the last preserves an unrecognised value rather than collapsing it |
| `health` | `Option<ContainerHealth>` | `Starting`, `Healthy`, `Unhealthy`; `None` when the runtime reports none — never defaulted to healthy |
| `ports` | `Vec<PublishedPort>` | may be empty |
| `created_at` | `String` (RFC 3339) | required |
| `started_at` | `Option<String>` (RFC 3339) | `None` when never started |
| `project` | `Option<String>` | the declared owning project; `None` means no project, never inferred (FR-007) |
| `cluster` | `Option<String>` | local cluster this container is a node of; label-sourced only (FR-042) |

`PublishedPort`: `{ host_ip: Option<String>, host_port: Option<u16>, container_port: u16, protocol: String }`.
`host_port` is `None` for an exposed-but-unpublished port.

**Invariant**: `state` is never inferred from `health` or vice versa. `Restarting` is a distinct
state the UI must render as such — the live daemon currently has a container in it.

---

## Project

| Field | Type | Constraint |
|---|---|---|
| `name` | `String` | the declared project name; the adoption key |
| `members` | `u32` | total containers |
| `running` | `u32` | `<= members`; both shown together (FR-008) |
| `definition_file` | `Option<String>` | label-sourced path; `None` when undeclared |
| `definition_state` | `DefinitionFileState` | `Present`, `Missing`, `Unreadable(String)`, `Undeclared` |
| `adoption` | `AdoptionState` | see below |

The no-project bucket is represented as a `Project` with `name: None`-equivalent handling: a
distinct `ProjectGroup::Unowned` variant rather than a project literally named "none", so no real
project can collide with it (FR-007).

**Invariant**: project-level operations are offered only when `definition_state == Present` **and**
`adoption == Adopted` (FR-032, FR-033). When `definition_state` is anything else the reason is
stated and the file is never written (Principle VI).

---

## Adoption

The persisted grant. Stored at `~/.fathom/container_adoption.json`.

| Field | Type | Constraint |
|---|---|---|
| `project` | `String` | project name; the key. Never a container id (FR-015) |
| `adopted_at` | `String` (RFC 3339) | when the operator granted it |

`AdoptionState` as carried in the protocol: `Adopted`, `NotAdopted`, or
`ImplicitViaManagedCluster` — the third for FR-016, so the UI can say *why* something is actionable
without an explicit grant rather than showing a bare `Adopted` the operator never gave.

### State transitions

```text
NotAdopted ──adopt──▶ Adopted ──release──▶ NotAdopted
```

- A container joining an adopted project is adopted with it; no transition occurs (FR-015).
- Removing a project's last container does not release it; the grant is on the name (edge case).
- `ImplicitViaManagedCluster` is computed, never stored, and cannot be released — releasing it
  would make a cluster Fathom created unmanageable (FR-016).
- Release during an in-flight operation does not cancel that operation; it takes effect for
  subsequent ones, and the in-flight outcome is still reported (edge case).

**Invariants**:

1. **Fail closed.** An unreadable, malformed or absent record yields the empty set (FR-047). The
   file is left on disk unmodified.
2. **One gate.** Every mutating command passes through a single adoption check. A new mutating
   command that forgets the gate must not compile — the gate takes the command, not a boolean.
3. Adoption never widens into another project: granting `A` affects no object whose project is `B`.

---

## RemovableObject

Images, volumes and networks share one shape, differing by `class` and by what `reclaimable` means.

| Field | Type | Constraint |
|---|---|---|
| `class` | `ObjectClass` | `Image`, `Volume`, `Network` |
| `id` | `String` | runtime identity |
| `display` | `String` | tag, volume name or network name |
| `tagged` | `Option<bool>` | images only; `Some(false)` is dangling. Never `None` for an image (FR-010) |
| `size_bytes` | `Option<u64>` | `None` when the runtime does not report it — not zero |
| `referenced_by` | `Vec<ObjectReference>` | containers and projects currently referencing it |
| `in_use` | `bool` | true when any reference is a running container |

`ObjectReference`: `{ kind: ReferenceKind, id: String, display: String, project: Option<String> }`
where `ReferenceKind` is `Container` or `Project`.

**Invariant**: `size_bytes: None` must render as "unavailable", never as `0` (FR-009, FR-037's
sibling rule for usage).

---

## OperationPreview

The heart of the feature. Built before anything is issued; the operator sees it; confirming issues
exactly what it describes.

| Field | Type | Constraint |
|---|---|---|
| `operation` | `String` | human label, e.g. "Stop container" |
| `command` | `Vec<String>` | the literal argv, program first, that will be executed. Not a prose rendering (FR-018) |
| `targets` | `Vec<ObjectReference>` | every object acted on, enumerated. Never a count (FR-023) |
| `destructive` | `bool` | true for any removal, any prune selection, any project-down |
| `removals` | `Vec<RemovalLine>` | non-empty exactly when `destructive`; one line per object |
| `reclaims_bytes` | `Option<u64>` | total space freed, when computable |
| `blast_radius` | `Vec<String>` | every project the operation reaches, including ones other than the selected (FR-024) |
| `excluded` | `Vec<ExcludedObject>` | objects left out for belonging to an unadopted project (FR-025) |
| `breaks_clusters` | `Vec<ClusterBreak>` | clusters that will break (FR-041) |

`RemovalLine`: `{ object: ObjectReference, size_bytes: Option<u64> }`.
`ExcludedObject`: `{ object: ObjectReference, reason: ExclusionReason }` where `ExclusionReason` is
`UnadoptedProject(String)`.
`ClusterBreak`: `{ cluster: String, connected: bool }` — `connected` true when the operator is
currently connected to it, which FR-041 requires stating separately from the break itself.

**Invariants**:

1. `command` is what runs. No code path may issue an argv that differs from the previewed one.
2. `destructive == true` ⟹ `removals` is non-empty and fully enumerated; a preview that cannot
   enumerate is not constructed, and the operation is not offered (FR-027).
3. `excluded` is populated *before* confirmation, not reported afterwards.

---

## OperationOutcome

| Field | Type | Constraint |
|---|---|---|
| `per_object` | `Vec<ObjectOutcome>` | one entry per previewed target, always |
| `overall` | `OutcomeStatus` | `Succeeded` only when every entry succeeded (FR-022) |
| `discrepancy` | `Option<Discrepancy>` | present when post-verification differs from the preview |

`ObjectOutcome`: `{ object: ObjectReference, status: OutcomeStatus, error: Option<String> }`.
`error` is the runtime's own stderr, verbatim and unwrapped (FR-021).
`Discrepancy`: `{ expected_removed: Vec<ObjectReference>, actually_removed: Vec<ObjectReference>, note: String }`.

**Invariant**: a destructive operation always runs post-verification and always reports a
discrepancy when one exists; silence is not an option (FR-023, SC-015).

---

## UsageSample

| Field | Type | Constraint |
|---|---|---|
| `container_id` | `String` | required |
| `cpu_percent` | `Option<f32>` | parsed from the runtime's formatted figure; `None` when unavailable |
| `memory_bytes` | `Option<u64>` | parsed; `None` when unavailable |
| `memory_limit_bytes` | `Option<u64>` | parsed; `None` when unavailable |
| `sampled_at` | `String` (RFC 3339) | required |

Aggregation per project sums members' `Some` values and states how many members' figures were
unavailable, rather than treating them as zero (FR-037).

History is bounded with an explicit cap, and reaching the cap is visible (FR-038). Protocol carries
numbers; the runtime's display strings ("24.56MiB / 31.25GiB", "0.00%") are parsed once in the
backend and never forwarded.

---

## Relationships

```text
RuntimePresence ──(Ready)──▶ Inventory
                                 │
        ┌────────────────────────┼─────────────────────────┐
        ▼                        ▼                         ▼
   ProjectGroup            RemovableObject            UsageSample
   (Project | Unowned)      (Image/Volume/Network)     (per container)
        │                        │
        │ members                │ referenced_by
        ▼                        ▼
    Container ◀───────────── ObjectReference
        │
        │ cluster (label-sourced)
        ▼
   local cluster ◀──FR-040── lists its backing containers

Adoption ──keyed by──▶ Project.name ──gates──▶ OperationPreview ──issues──▶ OperationOutcome
```

**Cross-cutting invariant**: every arrow into `OperationPreview` passes the adoption gate, and
every arrow out of it carries the same argv that the preview displayed.
