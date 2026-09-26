# Contract: Daemon HTTP API (v1)

Base path via aggregated API: `/apis/daemon.fathom.dev/v1` (paths below are relative to it). All endpoints are `GET`, read-only. The pod is ClusterIP-only and accepts requests only from the API server: it requires a client certificate signed by the requestheader CA and reads the caller from `X-Remote-User`/`X-Remote-Group`; requests without a verified identity get an empty scope (403 `noAccess`). `/healthz` and `/readyz` are served on a separate plain-HTTP kubelet-probe port.

| Path | Response | Notes |
|---|---|---|
| `/healthz` | 200 `ok` | liveness |
| `/readyz` | 200 when reflectors synced, else 503 | readiness |
| `/v1/capabilities` | `DaemonCapabilities` JSON | used for probe/handshake |
| `/v1/stream?epoch=&after=` | `application/x-ndjson` of `DaemonFrame` | first frame `hello`; then `truncated` if needed; replay; then live; `heartbeat` every 15 s |
| `/v1/topology` | `{ seq, nodes, edges }` | filtered to the caller's scope; deltas arrive on the stream as `topologyDelta` |
| `/v1/metrics?uid=&metric=&from=&to=` | `{ samples, truncated? }` | filtered per caller; `metricsUnavailable` if metrics API absent |
| `/v1/snapshot/{kind}` | `{ kind, resourceVersion, ageMs, synced, items: [...] }` | `kind` in `capabilities.snapshots` (`pods`, `events`); 404 otherwise; 403 `noAccess` if the caller may list it nowhere; 503 if any partition is not synced |

## Semantics
- `after` omitted: live-only from now (after `hello`). `after` present with matching epoch and `seq >= oldestSeq-1`: replay strictly greater than `after`. Otherwise emit `truncated` then live.
- Ordering: frames ordered by `seq`; no duplicates within an epoch.
- Errors: JSON `{ "error": "<verbatim message>" }` with 4xx/5xx; never HTML.
- Compatibility: additive fields allowed within a `protocolMajor`; clients ignore unknown frame types/fields; a higher `protocolMajor` yields `versionMismatch`.
- Filtering: every response and frame is filtered to the caller's `VisibleScope` (research R10); a permission change mid-stream produces `scopeChanged`. Identity is never taken from query parameters or client-supplied headers other than the verified front-proxy set.
- Snapshots: items come from the daemon's reflector stores, filtered to the caller's scope, sorted by (namespace, name). `resourceVersion` is the oldest per-partition version, so a client WATCH from it never misses a change (it may replay a few). `ageMs` is 0 while every watch is established and grows from the moment any watch dropped until it is re-established; a relist swaps the store atomically. The client uses a snapshot only when `ageMs < 30000`; otherwise, and on any error or timeout, it falls back to a direct LIST.
- Limits: max 8 concurrent streams (429 beyond); slow subscribers are dropped with `bye`.

## Client classification of probe responses
404 (group not served / APIService unavailable) or 503 (no endpoints) -> `notInstalled`; 403 -> `noAccess`; timeout/connection error -> `unavailable`; 200 with different major -> `versionMismatch`.
