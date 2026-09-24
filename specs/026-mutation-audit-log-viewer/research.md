# Research: Local Mutation Audit-Log Viewer

## R1: Where does a mutating command's completion get signaled today, and where should audit persistence hook in?

**Decision**: Hook into `run_cmd` in `src/backend.rs` (around the construction of
`BackendEvent::CommandFinished { ok, error }`, ~line 202). Snapshot an `AuditActionSummary`
(action label + target kind/name/namespace, derived from the `BackendCmdV1` value) **before**
`cmd::dispatch_cmd(cmd, ...)` consumes `cmd` by value, then write the audit entry immediately
after the `ok`/`error` outcome is known, using the same `route`/`mutation_target()` classification
`mutation.rs` already computes for `finished_route`.

**Rationale**:
- `src/protocol/v1/mutation.rs` does not itself emit any event; it only classifies
  `BackendCmdV1 -> MutationTarget` (`ReadOnly | Namespaced | ClusterScoped | Local | ContextSwitch`),
  consumed by `finished_route()` in `src/backend.rs` to route the terminal
  `BackendEventV1::CommandFinished { ok, error }`. That terminal event, emitted exactly once per
  command via `events::emit(...)`, is the single choke point every mutating (and non-mutating)
  command passes through — team-store publishes, Helm actions, node cordon/drain/taint, RBAC
  admin, YAML apply, all included. This matches the feature's explicit constraint: hook the
  existing emission point, don't build a second mutation-tracking mechanism.
- `CommandFinished` itself carries no action-type/target/actor/timestamp fields today (only
  `ok`/`error`); `MutationTarget` gives scope-shape (namespaces or cluster-scoped) but not a
  human-readable action label or resource name. A new, additive `audit_summary()` method
  (mirroring the existing exhaustive, no-wildcard-arm style of `mutation_target()`) is added to
  extract a label ("YAML Apply", "Helm Upgrade", "Node Cordon", etc.) and, where available, the
  target resource's kind/name/namespace from the command value — computed in the same place and
  at the same time as `mutation_target()`, before `cmd` is moved into `dispatch_cmd`.
- Only mutating actions are persisted (`MutationTarget::writes_cluster()` is true, plus `Local`
  mutations that change app state such as Helm/port-forward/MCP toggles are included per FR-001's
  "every mutating action"; pure `ReadOnly` and `ContextSwitch` commands are excluded, since they
  are not "mutating actions" against a target resource).
- Because the audit write happens after `dispatch_cmd` returns and outcome is known, it fits
  Constitution Principle I (report only after the backend reports the result) and FR-011
  (best-effort, never blocks the mutation — the write happens after outcome is already decided,
  and any I/O error during the audit write is swallowed with a `tracing::warn!`, never surfaced
  as the command's own failure).

**Alternatives considered**:
- Instrumenting each mutating command handler individually (e.g. inside
  `src/backend/cmd/workloads.rs`, `src/backend/cmd/team.rs`, Helm/RBAC handlers) — rejected: this
  is exactly the "second mutation-tracking mechanism" the feature explicitly says not to build,
  and it risks missed call sites as new mutating commands are added.
- Emitting the audit write as a new `SystemEvent` variant consumed by a frontend-side/store
  listener that persists on the UI side — rejected: violates Constitution Principle II ("All
  Kubernetes access, persistence... MUST live in the Rust backend... UI MUST NOT... read or write
  `~/.fathom/` directly").

## R2: What local persistence mechanism already exists outside the cluster, and should this feature reuse it?

**Decision**: Reuse the existing flat-JSON-file-under-`~/.fathom/` convention from
`src/backend/persistence.rs`, adding a new file `audit-log.json` (a bounded JSON array of
`AuditLogEntry`), read/written with the same `load_json_file`/`save_json_file`-style whole-file
read-modify-write helpers already used for `ports.json`, `cluster-settings.json`, `config.json`.

**Rationale**:
- The repo has **no embedded database** at all: no `rusqlite`, `sled`, or any DB crate in
  `Cargo.toml`/`Cargo.lock`. Every existing local-persistence feature (port forwards, UI config,
  integrations, credentials, and spec 014's team/cluster-settings store) uses plain JSON files
  under `~/.fathom/`, some with atomic temp-file+rename writes and optimistic-concurrency `rev`
  counters (`src/backend/team_store.rs`) for the *shared*, multi-writer team store, and simpler
  direct `fs::write` for *local*, single-writer-per-process files (`ports.json`,
  `cluster-settings.json`, `config.json`).
- The audit log is local-only, single-writer (one Fathom backend process owns `~/.fathom/` for
  its machine), append-mostly, and bounded (fixed retention — see R3), so it fits the *simpler*
  local-file pattern (`ports.json`/`cluster-settings.json` style: read whole array, mutate,
  write whole array), not the shared-store's rev-guarded atomic-rename pattern (that machinery
  exists specifically for concurrent writers across team members via a shared directory, which
  does not apply here).
- A bounded fixed-size JSON array (hundreds of entries, not unbounded) makes whole-file
  read-modify-write cheap enough to stay within the <100ms audit-write budget (SC-003) and avoids
  adding a new storage technology, which would need to be justified under Complexity Tracking and
  isn't warranted for this scale.
- This is an explicit, documented divergence check per the feature's own instruction ("reuse
  rather than inventing a new one, unless plan.md's research genuinely shows it's unsuitable —
  justify explicitly if you diverge"): **no divergence is needed.** The flat-JSON-file mechanism
  is suitable at the fixed-retention scale (v1 default: 500 entries, see R3); a future scale-up
  (e.g. multi-thousand-entry unbounded history) would be the trigger to revisit, but that is
  explicitly out of scope per the feature's non-goals (no retention-policy configuration UI).

**Alternatives considered**:
- SQLite via `rusqlite`/`sqlx` — rejected: no existing dependency, would be the first embedded DB
  in the codebase, and is unjustified complexity for a few-hundred-row bounded log with no query
  needs beyond in-memory filter/search over an already-loaded list.
- `sled` or another embedded KV store — same rejection as SQLite; also has known stability/
  maintenance caveats and would need a "pre-release crate" justification under the constitution's
  Development Workflow rule.
- Appending line-delimited JSON (JSONL) to an ever-growing file with lazy trimming — rejected:
  more complex to reconcile with a fixed retention bound (would need periodic compaction) versus
  a single bounded JSON array that is simply truncated to the newest N entries on every write;
  the existing codebase convention is single JSON-value-per-file, not JSONL.

## R3: What fixed retention default should v1 use?

**Decision**: Retain the most recent **500 entries**, trimmed (oldest-first eviction) on every
write that would exceed the bound. No time-based expiry in v1 (kept simple; count-based bound is
sufficient to keep the file small and the read/filter operations fast, and is easy to reason
about and test deterministically).

**Rationale**: 500 entries comfortably covers "at least 200 history entries" from SC-002 with
headroom, keeps the JSON file small (well under a few hundred KB even with verbose failure
reasons), and needs no background compaction job. This is a fixed implementation default per the
feature's explicit non-goal ("no retention-policy configuration UI... a sane fixed retention
default... is fine for v1 — document the choice").

**Alternatives considered**: Time-based retention (e.g. 30 days) — rejected for v1 as it requires
an extra periodic sweep and produces less predictable behavior for testing (SC-002's "200
entries" scenario is easiest to construct and assert against a count-based bound). Nothing
prevents adding a time-based secondary bound later as a non-breaking, additive change.

## R4: How does the frontend fetch and display the history?

**Decision**: Add one new read-only protocol command, `BackendCmdV1::RefreshAuditLog`, classified
as `MutationTarget::ReadOnly` (it only reads the persisted log, it does not itself mutate
anything), and one new `SectionKey` (`audit_log()` → `"audit-log"`, `_`-prefixed-analog scope
`Global` since it's not tied to a cluster/namespace). The backend responds with the existing
`BackendEventV1::ResourceList`-style event carrying `Vec<AuditLogEntry>`, following the same
list-view pattern used by other read-only resource lists (pods, events, etc.). The new UI viewer
pane (under `ui/src/components/panels/` or a new `ui/src/components/audit/`) requests
`RefreshAuditLog` on mount and applies filters/search client-side (consistent with existing
list-view filter/search patterns already in the Zustand store), since 500 entries is small enough
to filter entirely in memory with no backend-side pagination needed (SC-002's 10-second budget
is a UI responsiveness bar for client-side filtering, not a scaling requirement).

**Rationale**: Matches Constitution Principle II (one contract, protocol-defined command/event,
generated bindings) and Principle V (fast/quiet — filtering 500 in-memory entries is far under
the 100ms input-handling bar). No new backend push/streaming needed; the pane just re-requests
`RefreshAuditLog` after a `CommandFinished` for a mutating action is observed (or on a light
poll/manual refresh), avoiding a new watch/stream mechanism for a low-frequency data source.

**Alternatives considered**: Pushing a `SystemEvent::AuditLogEntryAdded` event on every mutation
so the frontend can append incrementally — considered but not required for v1: the feature's
scale (bounded, few-hundred-entry, human-triggered mutation frequency) does not need streaming
push; a simple re-fetch-on-demand keeps the change smaller and avoids a second event path to keep
in sync with the persisted file. Can be added later as a non-breaking enhancement if needed.
