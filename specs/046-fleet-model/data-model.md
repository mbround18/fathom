# Data model — 046 fleet model

Field constraints are stated here so `tasks.md` can quote them rather than leave them to
implementation-time discretion. Every type is defined once in Rust (Principle II); the TypeScript
in `ui/src/protocol/generated/` is generated from it and never hand-edited.

## Persisted shape — `~/.fathom/fleet.json`

```json
{
  "schema": 1,
  "members": { "<context name>": { "display_name": "…", "labels": { "k": "v" } } },
  "groups":  { "<group name>":   { "selector": "env in (prod,staging)" } },
  "adopted": { "<definition id>": { "rev": 7, "shared_by": "…", "adopted_at": "…", "still_shared": true } }
}
```

- `schema` is `u32`, currently `1`. New versions add fields only; a reader of schema `n` MUST read
  a file written at schema `n` and ignore unknown keys (FR-007).
- Unreadable or invalid → in-memory defaults, file left byte-for-byte untouched, and the verbatim
  parse reason reported to the UI (FR-007, Principle VI). Precedent: `load_team_config`
  (`src/backend/persistence.rs:684`).
- The file MUST NOT contain a credential, token or secret of any kind (FR-020). It holds context
  names, operator-authored strings and a selector expression; nothing else is permitted in it.
- Nothing else may be written here, and this must not be written anywhere else — not
  `cluster-settings.json`, not `federation.json`, not any per-context file (FR-008).

## `FleetMember` (protocol)

One kubeconfig context as the fleet knows it.

| Field | Type | Constraint |
|---|---|---|
| `context` | `String` | The key. The kubeconfig context name, verbatim. Never the API server URL; two contexts sharing one API server are two members (FR-001). Non-empty. |
| `display_name` | `Option<String>` | Operator-set. Trimmed; empty after trimming is stored as `None`, not `""`. Max 128 chars. Shown *beside* `context`, never instead of it (FR-002). |
| `labels` | `BTreeMap<String, String>` | Validated at entry (see **Label syntax**). Ordered map so the persisted file and every rendering are stable. |
| `origins` | `BTreeMap<String, SettingOrigin>` | Per-field origin, keyed by field name (`display_name`) or `labels/<key>`. Mirrors `ClusterSettingsFieldOrigin` (`src/protocol/v1/team.rs:44`). A local value wins and is marked local; the shared value it overrode stays visible (FR-013). |
| `availability` | `FleetMemberAvailability` | `Present` when the context is in the current kubeconfig, `Absent` when it is not. An `Absent` member keeps all metadata and is removed only by explicit operator action (FR-009). |

### Label syntax (FR-003)

Validated in the backend; the failing input and the rule it broke both appear in the error.

- **Key**: an optional DNS-subdomain prefix followed by `/`, then a name. Name: 1–63 characters,
  first and last alphanumeric, interior may also use `-`, `_`, `.`. Prefix: 1–253 characters,
  lowercase alphanumeric, `-` and `.`.
- **Value**: empty, or the same 1–63-character name shape as a key's name part.

## `FleetGroup` (protocol)

A named label selector over members. Membership is computed, never stored (FR-004).

| Field | Type | Constraint |
|---|---|---|
| `name` | `String` | Non-empty, trimmed, unique within the fleet, max 128 chars. |
| `selector` | `String` | The operator's expression, stored verbatim as typed. Parsed at entry; an invalid expression is rejected with the parse error verbatim and no group is created (FR-006). Round-trips to the string `ListParams::labels()` takes, which is what 047 consumes. |
| `origin` | `SettingOrigin` | Local or shared (FR-013). |

Supported grammar (the `kubectl -l` subset, parsed into `kube::core::Expression`): comma-separated
requirements; `k=v`, `k==v`, `k!=v`; `k in (a,b)`, `k notin (a,b)`; `k` (Exists); `!k`
(DoesNotExist).

A group matching no member is shown as matching nothing. It is not hidden, rejected or deleted
(FR-005). Members matching no group are reachable through an explicit ungrouped listing (FR-017).

## `FleetGroupRollup` (protocol)

A group's aggregated health. It always carries its own coverage — there is no variant of this type
that can state a sum without stating what the sum covers (FR-010, SC-004).

| Field | Type | Constraint |
|---|---|---|
| `group` | `String` | The group name this rolls up. |
| `members_total` | `u32` | Members the group's selector currently matches. |
| `members_covered` | `u32` | Members whose summary was read in full. `members_covered <= members_total` always. |
| `gaps` | `Vec<FleetCoverageGap>` | One per uncovered member. `gaps.len() == members_total - members_covered` is an invariant, asserted in test. |
| `node_ready` / `node_total` / `unhealthy_pods` | `u32` | Sums over the covered members only. Never rendered without `members_covered`/`members_total` beside them. |

### `FleetCoverageGap`

| Field | Type | Constraint |
|---|---|---|
| `context` | `String` | The member not covered. Named, not counted (FR-010). |
| `reason` | `FleetGapReason` | `NotPermitted { detail }` · `Unreachable { detail }` · `Absent` · `Pending`. |
| `detail` | in-variant `String` | The API server's own reason, verbatim. |

`NotPermitted`, `Unreachable` and `Absent` are three distinct variants and MUST NOT collapse into
one (FR-011). This is the type that stops the `integrationVisible` mistake — a denied read
rendered as "not installed" — from being repeated: there is no way to express "nothing found"
without saying whether it was looked at.

## `SharedFleetDefinition` (protocol, 014 store)

The publishable form of labels and groups. Attributes mirror `ConnectionProfile`
(`src/protocol/v1/team.rs:137`) because FR-012 says to reuse 014's semantics, not invent new ones.

| Field | Type | Constraint |
|---|---|---|
| `id` | `String` | Stable id within the store. |
| `members` | `BTreeMap<String, FleetMemberShare>` | Display name + labels only. |
| `groups` | `BTreeMap<String, String>` | Group name → selector. |
| `rev` | `u64` | Optimistic concurrency. A stale publish is refused with `StoreError::Conflict`, never silently overwritten (`src/backend/team_store.rs:1-14`). |
| `shared_by` | `String` | Attribution, as 014 records it. |
| `updated_at` | `String` | RFC 3339. |
| `removed` | `bool` | `true` once removed upstream. The row is **kept**, so an operator who adopted it is told "no longer shared" instead of losing it (FR-014). |

Store file: `fleet-definitions.json` in the configured store root, `{ "rev": u64, "definitions":
{ "<id>": SharedFleetDefinition } }` — the `ProfilesFile` shape, written atomically (temp +
rename).

### `AdoptedFleetDefinition` (local)

| Field | Type | Constraint |
|---|---|---|
| `definition` | `SharedFleetDefinition` | The copy as adopted. |
| `adopted_at` | `String` | RFC 3339. |
| `still_shared` | `bool` | `false` when the store is unreachable or the definition was removed upstream. The local copy is **never** deleted in either case (FR-014). |

## State transitions

**Member availability**: `Present` ⇄ `Absent` on each `RefreshContexts`, driven purely by whether
the context appears in the kubeconfig. Metadata is untouched by the transition. `Absent → removed
from the file` happens only on `ForgetFleetMember` (FR-009).

**Value origin**: `Shared → Local` on an operator edit, keeping the shared value visible as the
overridden one. `Local → Shared` only by an explicit discard-local action. Adoption never
overwrites a `Local` value (FR-013).

**Adoption**: `adopted → still_shared: false` when the store is unreachable or the upstream
definition is `removed`. There is no path from any of these to deletion of the local copy.

## What this model does not do

No pollers, clients or watches are created by anything here (FR-015). Fleet metadata is read at
launch and on explicit refresh, and written on operator edit — which is why SC-005 can claim zero
additional idle CPU and zero additional API server requests.
