# Contract: Rust ⇄ Webview Bridge

**Plan**: [../plan.md](../plan.md) · **Decision**: research R1, R2

## Surface

The webview sees exactly two Tauri commands. Both are registered in `fathom-desktop` via `generate_handler!` and allowed by `capabilities/default.json`. No other app commands are exposed.

| Command | Args | Returns | Semantics |
|---|---|---|---|
| `send_cmd` | `{ cmd: BackendCmd }` | `Result<(), BridgeError>` | Forward `cmd` to the backend's crossbeam sender. Returns an error only if the backend has stopped (`BridgeError::BackendGone`). It never waits for the command's result, because results arrive as events, as they do today. |
| `subscribe_events` | `{ onBatch: Channel<EventBatch> }` | `Result<SubscriptionInfo, BridgeError>` | Attaches the channel to the pump. **One subscriber at a time.** A new subscription replaces the old one, which handles a webview reload. Returns `{ protocolVersion: 1, backendStarted: bool }`. |

```ts
// ui/src/protocol/transport.ts
export interface Transport {
  send(cmd: BackendCmd): Promise<void>;
  subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo>;
}
```

`tauri-transport.ts` implements this with `invoke` + `new Channel()`. `fake-transport.ts` implements it in-page for dev (`VITE_TRANSPORT=fake`) and for Playwright.

## Types

All types are generated into `ui/src/protocol/generated/` by ts-rs. Nobody edits them by hand.

- `BackendCmd` = `BackendCmdV1`, an adjacently tagged enum: `{ "type": "RefreshPods", "data": { ... } }`. Unit variants serialize as `{ "type": "Shutdown" }`.
- `BackendEventEnvelope` = `{ version: 1, seq: u64, route: Route, correlation: Correlation, event: BackendEvent }`.
- `BackendEvent` = `{ type: "System" | "ResourceList" | "ResourceRead" | "Stream", data: ... }`. Each inner enum is also adjacently tagged.
- `EventBatch` = `{ envelopes: BackendEventEnvelope[], dropped: u32 }`. `dropped` counts envelopes the pump discarded because of backpressure. It should normally be 0, and the UI shows a warning in the status bar when it isn't.
- `u64` fields (`seq`, `RequestId`, `StreamId`) are exported as `number`. That's safe below 2^53, and they're monotonic counters that start at 0. A test asserts the counters' type limits.

## Pump behavior

1. A dedicated thread blocks on `gui_rx.recv()`. There is no polling and no timer while idle.
2. After the first envelope it drains more with `try_recv()` until **256 envelopes** or **16 ms** have passed, then sends one `EventBatch`.
3. If no subscriber is attached, envelopes are buffered up to 10,000. Beyond that the oldest `ResourceList` envelopes are dropped first, `Stream` next, and `System` never. The count is reported in `dropped`.
4. If `Channel::send` fails because the webview is gone, the subscriber is detached and buffering resumes.
5. `ctrlc` and window-close both send `BackendCmd::Shutdown`, wait up to 500 ms, and exit (same as today).

## Ordering and staleness

- Envelopes are delivered in backend `seq` order. Batches never reorder them.
- The UI drops an envelope when:
  - its `route` context doesn't match the active context, or
  - its `correlation.request_id` is older than the latest request for the same `(route, kind)` (FR-016).

## New commands (added to BackendCmdV1 by this feature)

| Variant | Purpose | Replaces |
|---|---|---|
| `LoadUiConfig` | → `SystemEvent::UiConfigLoaded { ui: Option<UiConfig>, expanded_groups: Vec<String>, mcp_auto_start: bool }` | Direct `persistence` reads in `views/navigation.rs` and `views/mcp_integration.rs` |
| `SaveUiConfig { ui: UiConfig, expanded_groups: Vec<String> }` | Persist nav expansion, layout profiles, and theme prefs to `config.json → ui` | Direct `persistence` writes |
| `SetMcpAutoStart { enabled: bool }` | Persist the MCP auto-start flag | The direct write in `mcp_integration.rs` |

`UiConfig` is defined in [../data-model.md](../data-model.md). The existing `config.json` keys (`navigation.expanded_groups`, `auto_start.mcp_http`) keep their meaning and location. `ui.*` is additive and falls back to defaults when absent, so there is no migration step (SC-009).

## Verification

- A Rust test round-trips every `BackendCmdV1` and `BackendEventV1` variant through `serde_json` and back.
- The ts-rs export runs in CI, and `git diff --exit-code ui/src/protocol/generated` must be clean.
- A Vitest contract test loads the recorded fixtures in `ui/tests/fixtures/*.json` and type-checks them against the generated types (`satisfies EventBatch`).
