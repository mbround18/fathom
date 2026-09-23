# Contract: Context Status Protocol & UI Surface (P1)

## Protocol (`src/protocol/v1/event.rs`, version 1 — additive, no bump)

```jsonc
// SystemEvent::ContextStatusUpdated
{
  "type": "ContextStatusUpdated",
  "data": {
    "context": "prod",
    "status": "Error",          // "Connected" | "Connecting" | "Error" | "NotConnected"
    "error": "connection refused"  // present only when status == "Error"
  }
}
```

Delivered inside a `BackendEventEnvelopeV1` on `Route::global(section_keys::system())`, exactly like
`ContextsUpdated`/`ContextChanged`. No new command is introduced — probing is triggered implicitly by the
existing `RefreshContexts`/`Initialize`/`SetContext` commands (no separate "RefreshContextStatuses" command
for P1, keeping the surface minimal).

## UI test hooks (`federation-` prefix, per task brief)

| `data-testid` | Where | Notes |
|---|---|---|
| `federation-context-switcher` | Container `<nav>` | Always mounted (no open action needed) below `TopBar`. |
| `federation-context-item` | One per context | `data-context-status="connected\|connecting\|error\|not_connected"`, `data-context-name`. `aria-pressed` true for the active context. Clicking sends `SetContext`. |
| `federation-context-status-dot` | Inside each item | Purely decorative status indicator; `aria-hidden`. |
| `federation-context-error` | Inside an `error`-status item only | Contains the verbatim backend error text; present in the DOM without requiring a switch attempt (FR-004) — satisfies "available on inspection" via both a native `title` tooltip and this always-queryable element. |

## Backend function contract (`src/backend/context_status.rs`, new module)

```rust
/// Fire-and-forget: spawns one bounded probe task per `contexts` entry other than `active`.
/// Each task emits Connecting immediately, then NotConnected or Error{detail} once its probe
/// (bounded by a 5s timeout) settles. Never touches `state.cluster.client` (P1 keeps exactly one
/// live client — the active context's).
pub(super) fn spawn_status_probes(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    contexts: &[String],
    active: Option<&str>,
);
```

## Backend call-site contract (`src/backend/cmd/cluster.rs`)

- `initialize()`: emit `ContextStatusUpdated{context: <target>, Connecting}` immediately before
  `connect_and_share_client`; emit `Connected` or `Error{detail}` immediately after, for whichever context
  ends up active (the explicit selection, or kubeconfig's `current-context`).
- `set_context()`: same Connecting-before / Connected-or-Error-after bracketing around
  `connect_and_share_client`, for the newly-selected context.
- Both call sites, after `load_and_send_contexts` returns the freshly loaded `(contexts, current)`, call
  `context_status::spawn_status_probes(state, sender, &contexts, current.as_deref())`.
