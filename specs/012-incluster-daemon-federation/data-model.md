# Phase 1 Data Model: P1 Multi-Context Switcher & Live Status

## `ContextConnectionStatus` (new, `src/protocol/v1/event.rs`)

Fieldless enum, serialized as a bare string (matches `NotificationLevel`'s convention):

| Value | Meaning |
|---|---|
| `Connected` | This is the active context and its live client is connected. |
| `Connecting` | A connect attempt (active context) or reachability probe (other context) is in flight. |
| `Error` | Last connect attempt or probe failed; `error` on the event carries the verbatim message. |
| `NotConnected` | Reachability probe succeeded but this is not the active context (no live client). |

State transitions (per context, independent of other contexts):

```
(no event yet) --Initialize/RefreshContexts/SetContext--> Connecting
Connecting --probe or connect succeeds--> Connected (active context only) | NotConnected (other contexts)
Connecting --probe or connect fails-----> Error{error}
Connected/NotConnected/Error --SetContext switches which context is active--> Connecting (re-evaluated)
```

Only one context is ever `Connected` at a time in P1 (the single active-client model, unchanged). This is
a P1 invariant, not enforced by the protocol type itself — P2 will need to revisit it when several
contexts can be simultaneously live.

## `SystemEvent::ContextStatusUpdated` (new variant)

```rust
ContextStatusUpdated {
    context: String,
    status: ContextConnectionStatus,
    error: Option<String>,
},
```

- Routed `Route::global(section_keys::system())`, matching `ContextsUpdated`/`ContextChanged`.
- `error` MUST be `None` unless `status == Error` (enforced by construction at the two emission sites,
  covered by a unit test).
- Emitted:
  - Once per non-active context, per contexts-load, by `context_status::spawn_status_probes` (`Connecting`
    immediately, then `NotConnected`/`Error{..}` when the probe settles or times out).
  - For the active context only, by `cmd/cluster.rs`'s `initialize()`/`set_context()` (`Connecting`
    immediately before connecting, then `Connected`/`Error{..}` immediately after).

## Frontend: `SessionState.contextStatuses` (new field, `ui/src/store/index.ts`)

```ts
type ContextStatusEntry = { status: ContextConnectionStatus; error?: string }
// SessionState:
contextStatuses: Record<string, ContextStatusEntry>
```

- Updated only by the new `ContextStatusUpdated` handler case in `ui/src/store/handlers/system.ts`: last
  write per context name wins (matches the backend's single-authoritative-writer-per-context design in
  research.md R3 — no client-side reconciliation needed).
- Not persisted (matches `session.contexts`/`activeContext`, which are also runtime-only, re-populated from
  the backend on every launch).
- A context absent from the map (never probed yet, e.g. right after `ContextsUpdated` arrives before any
  probe settles) renders as `not connected` in the UI by convention of the component (no map entry ==
  `NotConnected` display state), not a fifth wire status.

## Key Entities cross-reference (spec.md)

- **Kubeconfig Context**: unchanged shape (`session.contexts: string[]`), now paired with
  `contextStatuses[name]`.
- **Active Context**: unchanged (`session.activeContext`); its status entry is always driven by the
  connect/switch path per R3, never by a background probe.
- **Cluster Connection**: modeled as the `(status, error)` pair in `contextStatuses`; the P4 "optional link
  to daemon enrichment state" mentioned in spec.md's Key Entities is explicitly not modeled here (deferred
  with P4).
