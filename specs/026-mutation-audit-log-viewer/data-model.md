# Data Model: Local Mutation Audit-Log Viewer

## AuditLogEntry

Persisted record of one completed mutating action. Serialized as a JSON array element in
`~/.fathom/audit-log.json`. Defined in Rust (`src/protocol/v1/audit.rs`, new file) and exposed to
the UI via generated TypeScript bindings, per Constitution Principle II.

| Field | Type | Notes |
|---|---|---|
| `id` | `String` (UUID v4) | Unique per entry; generated at write time. |
| `timestamp` | `String` (RFC 3339 / ISO 8601, UTC) | Command-completion wall-clock time, captured at the audit-write hook point (`run_cmd`). |
| `action` | `AuditActionKind` (enum, see below) | Coarse action category, derived from the `BackendCmdV1` variant via `audit_summary()`. |
| `action_label` | `String` | Human-readable label for display (e.g. "Node Cordon", "Helm Upgrade", "Apply YAML"), derived alongside `action`. |
| `context` | `String` | The kubeconfig context the action targeted (from backend cluster state at dispatch time). |
| `target_kind` | `Option<String>` | Resource kind (e.g. `"Node"`, `"HelmRelease"`, `"ClusterRoleBinding"`), when determinable from the command. |
| `target_namespace` | `Option<String>` | Namespace, when the target is namespaced. |
| `target_name` | `Option<String>` | Resource name, when determinable. |
| `outcome` | `AuditOutcome` (enum: `Success` \| `Failure { reason: String }`) | Mirrors `CommandFinished { ok, error }`. |
| `request_id` | `Option<String>` | The originating `RequestId` (correlation), for cross-referencing with in-session events; best-effort. |

**Validation rules**:
- `id`, `timestamp`, `action`, `action_label`, `context`, `outcome` are always present.
- `target_kind`/`target_namespace`/`target_name` are independently optional — a cluster-scoped
  action may have `target_kind`+`target_name` but no `target_namespace`; a `Local` mutation (e.g.
  MCP toggle) may have none of the three.
- `outcome = Failure` MUST carry a non-empty `reason` when the originating `CommandFinished.error`
  was `Some(..)`; if the error was absent but the command is otherwise known to have failed, use a
  generic placeholder reason rather than omitting it (never leave `Failure` reason-less if avoidable).

**State transitions**: None — entries are immutable once written (append-only); the only mutation
to an existing entry set is truncation (oldest-first eviction) when the retention bound (500
entries, see `research.md` R3) is exceeded by a new append.

## AuditActionKind (enum)

Closed, exhaustive enum (no wildcard arm, matching the existing `mutation_target()` style) mapping
1:1 (or many:1) from `BackendCmdV1` mutating variants to a stable, serializable category, used for
the "filter by action type" requirement (FR-004). Examples: `YamlApply`, `HelmUpgrade`,
`HelmRollback`, `HelmUninstall`, `NodeCordon`, `NodeUncordon`, `NodeDrain`, `NodeTaint`,
`RbacChange`, `BulkDelete`, `Delete`, `Scale`, `Restart`, `PortForwardChange`, `McpToggle`,
`TeamProfilePublish`, `ContextConfigChange`, etc. (exact enumeration finalized during
implementation from the full `BackendCmdV1` match in `mutation.rs`/`cmd.rs`).

## AuditLogStore (backend-internal, not part of the wire protocol)

The on-disk container: `{ "entries": [AuditLogEntry, ...] }` in `audit-log.json`, newest-last or
newest-first (implementation choice; viewer sorts explicitly regardless). Bounded to 500 entries
(oldest evicted first). Loaded/saved via the same `load_json_file`/`save_json_file` helpers as
other `~/.fathom/*.json` files (`src/backend/persistence.rs`).

## AuditLogFilter (frontend-only, transient UI state)

Not persisted (per spec's Key Entity "Audit Log Filter/Search State — not persisted beyond the
current viewing session"). Shape (Zustand store slice):

| Field | Type | Notes |
|---|---|---|
| `actionKinds` | `AuditActionKind[]` | Empty = no filter. |
| `outcome` | `"all" \| "success" \| "failure"` | Default `"all"`. |
| `context` | `string \| null` | Default `null` (all contexts). |
| `timeRange` | `{ from: string \| null; to: string \| null }` | Default unset (all time, within retention). |
| `searchText` | `string` | Case-insensitive substring match against `target_name` (and `target_kind`/`action_label` as a reasonable UX extension). |
