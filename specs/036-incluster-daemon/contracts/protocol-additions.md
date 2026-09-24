# Contract: Fathom protocol additions (additive, no version bump)

Defined in Rust in `src/protocol/v1/`; TS bindings regenerated into `ui/src/protocol/generated/`.

## Commands (`cmd.rs`)
- `SetDaemonPreference { context: String, mode: DaemonMode }` — persists preference, triggers probe when `auto`, tears down stream when `off`.
- `RefreshDaemon { context: String }` — manual re-probe (resets backoff).
- `RequestDaemonTimeline { context: String, limit: u32 }` — returns already-buffered client-side timeline (no new server call).

## Events (`event.rs`)
- `SystemEvent::DaemonStatusUpdated { context, status: DaemonStatus }` — keyed globally by context, mirroring `ContextStatusUpdated`.
- `SystemEvent::DaemonEventsBatch { context, session, events: Vec<DaemonTimelineEntry>, truncated: Option<TruncationMarker> }` — batched (max 200 per batch / 250 ms) and tagged with the session id so superseded sessions are discarded.
- `DaemonTimelineEntry { seq, time, event: DerivedControllerEvent }`.

## Added by clarification 2026-09-24
- Command `RequestDaemonTopology { context }` and `RequestDaemonMetrics { context, uid, metric, range }`.
- Events `SystemEvent::DaemonTopologyUpdated { context, session, delta }` and `SystemEvent::DaemonMetricsUpdated { context, session, series }`; `DaemonStatus.state` unchanged, `detail` carries the verbatim `noAccess`/identity error.
- New nested types: `TopologyNode`, `TopologyEdge`, `MetricSample`, `SeriesKey`, `VisibleScope`, frames `scopeChanged`/`topologyDelta`. Round-trip tests for each.

## Tests required (Principle III)
- Serialization round-trip for each new command/event variant and nested type.
- Generated-binding drift check remains green.
- UI: Playwright against the scripted fake backend covering states `off`, `notInstalled`, `available`, `unavailable`, `noAccess`, `versionMismatch`, the timeline, and the truncation banner; Vitest for timeline merge/dedupe and staleness discard.
