# Phase 0 Research: Agent SDK Ops

## R1: Model client — no official Rust "Claude Agent SDK" package exists

**Decision**: Implement the backend agent client directly against Anthropic's public
**Messages API** (`POST /v1/messages`, including its documented tool-use / streaming
semantics) using `reqwest`, which is already a workspace dependency. Do **not** add a
new crate.

**Rationale**: The "Claude Agent SDK" (formerly Claude Code SDK) is published by
Anthropic only as TypeScript (`@anthropic-ai/claude-agent-sdk`) and Python
(`claude-agent-sdk`) packages; there is no official or Anthropic-maintained Rust
package, and Principle II requires all model calls to live in the Rust backend, not the
TS/React UI. Reusing the TS SDK from the frontend would violate Principle II (UI calling
an LLM provider directly) and would require exposing the API key to the renderer
process, violating FR-004/Principle VI. The Messages API is the same public interface
the Agent SDK itself wraps (system prompt + tool-definitions + tool-result turns); a
thin Rust client implementing that same request/response shape and turn loop satisfies
FR-005 ("integrate ... exclusively through the Claude Agent SDK's public interface, not
internal/undocumented APIs") in spirit and in practice — no undocumented endpoint is
used, and swapping in an official Rust SDK later (if one ships) is a drop-in client
replacement behind the same `AgentClient` trait (see data-model.md), not a rewrite of
grounding/orchestration/confirmation logic.

**Alternatives considered**:
- *Third-party community `anthropic-sdk` Rust crates on crates.io* — rejected: none are
  Anthropic-maintained, several are unmaintained or version-lagging, and adding an
  unaudited third-party crate for API-key-bearing HTTP calls is a worse supply-chain
  posture than using `reqwest` (already vetted, already a dependency) directly against
  Anthropic's documented, versioned REST contract.
- *Shelling out to the Node/Python Agent SDK as a subprocess from Rust* — rejected: adds
  a Node/Python runtime dependency to a native desktop app, complicates packaging across
  all three Tier 1 platforms, and adds an IPC surface with no corresponding safety
  benefit over a direct REST client.
- *Running the TS Agent SDK inside the Tauri webview (frontend)* — rejected outright:
  directly violates Principle II and FR-002.

## R2: Local Ollama integration

**Decision**: Use `reqwest` against the local Ollama REST API (`POST /api/chat` on the
operator-configured endpoint, default `http://localhost:11434`). No Ollama client crate
is added (same supply-chain reasoning as R1); Ollama's HTTP API is small, stable, and
already matches the shape our `AgentClient` trait needs (chat turns with tool-call
results).

**Rationale**: Matches "self-hosted-first, no forced cloud dependency" and keeps the
backend's only new runtime dependency on functionality (`reqwest`) that is already
audited and present.

**Alternatives considered**: `ollama-rs` (community crate) — rejected for the same
supply-chain reason as R1; revisit only if the direct client proves materially more
code than is worth maintaining.

## R3: Grounding data source — no new ingestion path

**Decision**: The agent orchestration layer (`src/backend/agent/grounding.rs`) reads
directly from the same in-memory backend state that already feeds
`BackendEventV1::PodDetailsUpdated`/`EventsUpdated` (events, conditions, container
statuses, lifecycle fields) and the existing bounded log-tail buffer used by
`StartPodLogs`. It performs no separate Kubernetes API calls and opens no new watches.

**Rationale**: FR-001 explicitly forbids "a separate scraping, polling, or ingestion
path bypassing the existing store." Reusing already-streamed state also means grounding
data is bounded/truncated exactly as Principle V already requires (ring buffers), so
FR-018 (bounded + indicated truncation) is satisfied by construction — truncation
markers already present in the log/event buffers are forwarded into the grounding
summary the model sees.

## R4: Credential storage for the BYOK Anthropic key

**Decision**: Store the BYOK key (and the Ollama endpoint/model config) in the existing
`~/.fathom/credentials.json` file, under a new top-level `agent` key, using the exact
same `load_credentials()`/`save_credentials()` functions in
`src/backend/persistence.rs` already used for other stored credentials (e.g. registry
secrets). No new encryption mechanism is introduced.

**Rationale**: Direct inspection of the current codebase shows `credentials.json` is
persisted via `save_json_file`/`load_json_file` (plain JSON on disk in `~/.fathom/`,
which the OS already restricts to the user's own account via normal filesystem
permissions on all three Tier 1 platforms) — the `// Stored encrypted` comment on
`IntegrationConfig::api_key` in `src/backend/integrations.rs` is aspirational/no
at-rest cipher is currently implemented anywhere in the codebase (`ring` is present as a
dependency but is used for TLS, not for encrypting `credentials.json`). This feature
therefore follows the codebase's actual, existing precedent exactly rather than
introducing an inconsistent, feature-specific encryption scheme: the key lives in
`credentials.json` alongside other credentials, is never written to `tracing::*` logs,
is never included in a `BackendEventV1` payload in plaintext (the UI only ever receives
a masked/redacted form, revealed only via an explicit "reveal" command that echoes the
plaintext back once, on demand, the same way other credential-reveal flows in this
codebase behave), and an unreadable/corrupt `credentials.json` falls back to
"not configured" without deleting the file (Principle VI). If the project later adopts
real at-rest encryption for `credentials.json` project-wide, this feature inherits it
automatically with no format change, since it uses the shared file and functions.

**Alternatives considered**: OS keychain integration (`keyring` crate) — rejected for
v1: no existing credential in this codebase uses the OS keychain, so adding it here
would be a new, feature-specific storage convention rather than following the codebase's
existing one (research goal per task brief: "match whatever secret-handling convention
already exists"), and it adds a new dependency + per-platform code path this feature
does not need to justify on its own. Worth revisiting as a cross-cutting hardening pass
for all of `credentials.json`, not scoped to this feature.

## R5: Reusing the existing confirmation/diff-preview mechanism

**Decision**: `SuggestedAction` carries only `{ action_kind, target: ObjectRef, params }`
(e.g. `{action_kind: "restart", target: {kind: "Pod", namespace, name}}`). The frontend
maps `action_kind` to the *same* `ActionDescriptor` already registered for that
resource kind in `ui/src/resources/*.tsx` (built with the existing `deleteAction`/
`restartAction`/`scaleAction` helpers in `ui/src/resources/actions.ts`) and opens the
*same* `ConfirmDialog` component (`ui/src/components/ConfirmDialog.tsx`) already used
for manual invocations, calling that descriptor's existing `plan()` function against the
row's *current* store state at review-open time (never a snapshot cached from when the
suggestion was generated). Confirming calls the existing `runAction`/command-dispatch
path unchanged. For Helm-specific actions (rollback/upgrade), the existing
`PreviewHelmRollback`/`ApplyHelmRollback` two-step pattern is reused as-is — the
assistant only ever triggers the *preview* step on "review," identical to a manual
button click.

**Rationale**: This is a direct implementation of FR-008/FR-009/FR-010 and the "no
parallel mutation path" requirement; it also means zero new confirmation UI code is
needed, only a small adapter mapping `action_kind` -> existing descriptor lookup, which
is unit- and E2E-testable in isolation (does the adapter ever call a *mutating* command
directly, or does it only ever call `plan()`/open the dialog?).

## R6: Suggested vs Executed Action representation (protocol-level separation)

**Decision**: `SuggestedAction` and `ExecutedAction` are distinct Rust enums/structs in
`src/protocol/v1/agent.rs`, serialized as distinct, differently-tagged JSON shapes (not
a shared struct with an `executed: bool` flag). An `AgentMessage` holds
`Vec<SuggestedAction>` for proposals it made; a conversation separately accumulates
`Vec<ExecutedAction>` records, each referencing the `SuggestedAction` it came from by id
plus the resulting command outcome. There is no code path that mutates a
`SuggestedAction` in place into an `ExecutedAction` — a *new* `ExecutedAction` record is
appended once the backend receives the operator-confirmed command and its result comes
back on the normal `CommandFinished`/error path. This directly satisfies FR-006's "MUST
NOT be serialized or treated identically."
