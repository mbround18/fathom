# Data Model: In-Cluster Daemon

All types live in `src/protocol/v1/daemon.rs` (Rust, `serde` + `ts-rs`), shared by the daemon and the client.

## DaemonCapabilities (response of `/v1/capabilities`)
- `protocolMajor: u32`, `protocolMinor: u32`, `daemonVersion: string`
- `epoch: string` (random id per process start)
- `kinds: string[]` (watched kinds), `scope: { mode: "cluster" | "namespaces", namespaces: string[] }`
- `retention: { maxEvents: u32, maxAgeSecs: u64 }`
- `snapshots: string[]` (kinds with warm snapshot support; v1 `["pods","events"]`)
- `synced: bool` (all reflectors initialized)

## EventCursor
`{ epoch: string, seq: u64 }` — client persists last cursor per context in memory only for the session and in `~/.fathom/daemon.json` (additive file) so it survives app restarts.

## DaemonFrame (NDJSON line, tagged by `type`)
- `hello { capabilities }`
- `event { seq, time, event: DerivedControllerEvent }`
- `truncated { oldestSeq, oldestTime, dropped: u64, reason: "epochChanged" | "cursorTooOld" }`
- `heartbeat { seq, time }`
- `bye { reason }`

## DerivedControllerEvent
- `origin: "derived" | "raw"`
- `kind: string` (e.g. `Deployment`), `namespace: string?`, `name: string`, `uid: string`
- `category: "rollout" | "condition" | "scaling" | "job" | "cronjob" | "pod" | "event"`
- `severity: "info" | "warning" | "error"`
- `reason: string`, `message: string` (verbatim from status/Event)
- `repeat: u32` (coalesced count, >= 1)
- Validation: `name` non-empty; `message` capped at 4 KiB (with `…truncated` marker); never contains Secret data.

## DaemonStatus (client-side, per context; emitted as `SystemEvent::DaemonStatusUpdated`)
- `context: string`
- `state: "off" | "notInstalled" | "probing" | "available" | "unavailable" | "noAccess" | "versionMismatch"`
- `detail: string?` (verbatim error), `capabilities: DaemonCapabilities?`
- `lastEventAt: time?`, `nextRetryAt: time?`
- State transitions: off -> probing (pref auto + connect) -> {available | notInstalled | noAccess | unavailable | versionMismatch}; available -> unavailable (stream drop) -> probing (backoff); any -> off (pref off).

## DaemonPreference (client config, additive)
`{ context: string, mode: "off" | "auto" }`; missing entry = `off`.

## DaemonConfig (daemon env/flags)
`FATHOM_DAEMON_LISTEN` (default `0.0.0.0:8080`), `FATHOM_DAEMON_MAX_EVENTS` (10000), `FATHOM_DAEMON_MAX_AGE` (24h), `FATHOM_DAEMON_NAMESPACES` (empty = cluster), `FATHOM_DAEMON_COALESCE_SECS` (5), `RUST_LOG`.

## EventLog (daemon, in-memory)
Ring buffer of `(seq, time, DerivedControllerEvent)`; invariants: seq strictly increasing, contiguous within an epoch; eviction by count or age; `oldestSeq` tracked; broadcast channel fans out live frames to subscribers with bounded per-subscriber queues (slow consumer gets `truncated` then resumes).

## Added by clarification 2026-09-24

### CallerIdentity (daemon, per request)
`{ user: string, groups: string[], extra: map }` parsed only from verified front-proxy headers; absent/unverified = empty scope.

### VisibleScope (daemon, derived, cached <= 60 s)
`{ identityKey: string, namespaces: string[], clusterScoped: bool, resolvedAt: time }` computed via `SubjectAccessReview`; drives filtering of events, topology and metrics. Fail closed.

### Frame additions (`DaemonFrame`)
- `scopeChanged { namespaces: string[] }` (visible scope changed mid-stream)
- `topologyDelta { seq, added: TopologyNode[], removed: uid[], edges: EdgeDelta }`

### TopologyNode / TopologyEdge
- Node: `{ uid, kind, namespace?, name, health: "ok" | "warning" | "error" | "unknown" }`
- Edge: `{ from: uid, to: uid, relation: "owns" | "selects" | "scheduledOn" | "scales" }`
- Invariants: edges only between nodes visible to the caller; graph changes carry the shared seq.

### MetricSample / SeriesKey
- `SeriesKey { objectUid, metric: "cpu" | "memory" }`; `MetricSample { time, value: f64 }`.
- Ring: fixed cap per series (~240 at 15 s), total bytes bounded; query returns samples plus optional `truncated { oldestTime }`.
- Capabilities gain: `topology: bool`, `metrics: "available" | "metricsUnavailable"`, `perUserFiltering: true`.

### SeriesStore (trait, daemon)
`append(key, sample)`, `query_range(key, from, to) -> (samples, truncated)`. Only the in-memory ring implements it in 036.
