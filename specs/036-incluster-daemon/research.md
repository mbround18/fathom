# Research: In-Cluster Daemon

## R1. How does the desktop client reach the daemon?

**Decision (revised by clarification 2026-09-24)**: Aggregated API. The daemon is registered with an `APIService` (e.g. `v1.daemon.fathom.dev`), so the API server authenticates the caller with the context's existing credentials, RBAC-gates the group, and forwards a verified identity (`X-Remote-User`, `X-Remote-Group`, `X-Remote-Extra-*`) to the daemon over the front-proxy, mutually authenticated by the `requestheader-client-ca` bundle. Client path: `/apis/daemon.fathom.dev/v1/...` via the `kube` client.
**Rationale**: The plain service proxy strips `Authorization` and forwards no identity, so it cannot support per-user filtering (FR-017) (to be confirmed on kind in T041). An aggregated API is the Kubernetes mechanism for this; no ingress, port-forward, or client-held secret is introduced, and every kubeconfig auth mode incl. 035 exec plugins keeps working.
**Cost**: one cluster-scoped `APIService` object (removed by the uninstall overlay). Clusters that forbid `APIService` registration cannot use the daemon; the client shows `unavailable` with the cause. The daemon needs the `extension-apiserver-authentication` configmap (namespaced read in `kube-system`) to load the front-proxy CA.
**Alternatives**: (a) plain service proxy, no identity: rejected, leaks across tenants; (b) client sends its token in a custom header: rejected, hands user credentials to a pod (Principle VI); (c) port-forward: needs pods/portforward, fragile; (d) Ingress + token: duplicates 031.

## R2. Wire format

**Decision**: HTTP GET streaming NDJSON (`application/x-ndjson`), one JSON `DaemonFrame` per line, with heartbeat frames every 15 s; resume via `?after=<seq>&epoch=<id>`.
**Rationale**: Service proxy reliably passes chunked HTTP; WebSocket upgrade through the proxy is supported but less uniformly across managed control planes and intermediaries. NDJSON is trivially testable, and serde types are the same Rust types that `ts-rs` exports.
**Alternatives**: WebSocket (kept as a future optimization behind the capability handshake); gRPC (extra dependency, proxy pain).

## R3. Reuse of backend/protocol crates

**Decision**: New workspace member `daemon/` (bin `fathom-daemon`) depending on the root `fathom` lib with `default-features = false` (no egui). Reuse `kube` runtime watchers/reflectors, the protocol types in `src/protocol/v1`, and existing status/condition derivation helpers under `src/backend`. Add daemon-only types in a new protocol module `src/protocol/v1/daemon.rs` shared by daemon and client.
**Risk**: `fathom` lib currently exposes egui-gated modules; T003 verifies `cargo build -p fathom-daemon` pulls no egui/eframe/Tauri (check via `cargo tree`). If the lib drags UI deps, extract minimal shared modules rather than duplicating.
**Alternatives**: A separate `fathom-core` crate refactor (bigger, rejected for now); copy types (violates "one contract").

## R4. Event derivation

**Decision**: Derive from watch deltas by diffing prior vs. new status per object: Deployment/StatefulSet/DaemonSet rollout (`observedGeneration`, `updatedReplicas`, `availableReplicas`, Progressing/Available conditions incl. `ProgressDeadlineExceeded`), Job Complete/Failed, CronJob last-schedule and failed-job counts, HPA `currentReplicas`/desired changes and ScalingLimited, Pod Ready/restart-count/`CrashLoopBackOff`/OOMKilled transitions. Raw core/v1 Events are forwarded as kind `raw`. Derived events carry `reason` and `message` from status conditions verbatim.
**Rationale**: Matches "controller-level" in the roadmap; cheap; needs only watch access already required.
**Coalescing**: per (object, reason) max 1 per 5 s with `repeat` count.

## R5. Retention and replay

**Decision**: In-memory ring (default 10,000 events / 24 h), monotonic `seq` (u64) plus random `epoch` (per process start). Cursor = (epoch, seq). Mismatched epoch or seq below `oldest_seq` yields a `truncated` frame carrying `oldest_time` and `dropped` estimate, then live continues.
**Rationale**: Simple, bounded, no PV; restart loss is explicit rather than silent.
**Alternatives**: PVC-backed WAL (persistence is a non-goal; adds storage footprint).

## R6. Warm snapshots (Story 4)

**Decision**: Daemon exposes `GET /v1/snapshot/{kind}` from its reflector store with `resourceVersion` and `age`; client uses it only if `age < 30 s` and daemon `synced=true`, then applies deltas from the stream; otherwise direct LIST+WATCH. Pods and Events only in v1.
**Rationale**: Bounded scope; identical fallback path to today.

## R7. Security posture

**Decision**: Daemon ServiceAccount has ClusterRole with `get,list,watch` on the enumerated kinds only, excluding `secrets` and `configmaps`. Optional `FATHOM_DAEMON_NAMESPACES` allowlist switches to per-namespace Roles. Pod runs non-root, read-only rootfs, drops all caps, seccomp RuntimeDefault, distroless static image (shared with 031). NetworkPolicy allows ingress only from the API server (documented as best-effort; proxy traffic source varies by distribution, so default overlay ships it disabled). Payload redaction: derived events include names, reasons, messages and counts only.
**Per-user filtering**: see R10. The ServiceAccount's broad read is never exposed unfiltered.

## R8. Client detection and opt-in

**Decision**: Per-context `daemonPreference` (`off` default | `auto`). On connect with `auto`, one `GET /v1/capabilities` (5 s timeout) classifies: 404/503-no-endpoints -> `notInstalled`; 403 -> `noAccess`; network/timeout -> `unavailable`; protocol major mismatch -> `versionMismatch`; 200 -> `available` and opens the stream. Backoff 5 s -> 5 min cap on failure.
**Rationale**: Guarantees zero footprint and zero requests by default (Story 2).

## R9. kind packaging and testing

**Decision**: `k8s/addons/fathom-daemon/` (namespace, SA, ClusterRole/Binding, Deployment, Service) plus `k8s/overlays/daemon/`; `cluster.sh up-daemon` builds/loads the image with `kind load docker-image` and applies the overlay; `k8s/samples/daemon-events.yaml` holds a Deployment with a controllable rollout, a failing Job and a CronJob to generate events. Integration test script `k8s/test-daemon.sh` drives kubectl and asserts against the daemon stream with `kubectl get --raw`. Depends on 031 landing `k8s/` and `deploy/Dockerfile`; until then tasks create the same paths and merge-conflict resolution favors 031's files. Unit/integration in Rust use `tower-test` mock kube client for derivation and replay logic (no cluster needed).
**RustFS/MinIO**: not applicable (no object storage).

## R10. Per-user authorization (FR-017)

**Decision**: Identity only from verified front-proxy headers (reject requests whose client cert is not signed by the requestheader CA). Per request/stream, resolve the caller's visible namespaces: `SubjectAccessReview` `list` on each watched kind per namespace, cached per (user, groups-hash, namespace, kind) for <= 60 s; the daemon needs `create` on `subjectaccessreviews` (the only non-read verb, non-mutating, documented). Filter replay, live frames, topology and metrics with the same resolved set; re-resolve on cache expiry and drop newly-forbidden objects with a `scopeChanged` frame. Any error or missing identity yields an empty scope (fail closed).
**Rationale**: The shared buffer serves all users; filtering at read time avoids per-user buffers. SAR is the same RBAC source of truth as the API server, so Fathom never escalates.
**Risk**: SAR volume for many namespaces: batch by namespace, coalesce concurrent identical checks; bounded cache size.
**Alternatives**: per-user buffers (memory x users), per-tenant daemons (operational burden), unfiltered (rejected in clarification).

## R11. Live topology (FR-018)

**Decision**: Build the graph in-memory from the existing reflectors plus Nodes and Services: owner references (Deployment -> ReplicaSet -> Pod), Service selector -> Pod, Pod -> Node, HPA -> target, Job/CronJob ownership. Nodes carry `{kind, namespace, name, uid, health}`; edges `{from, to, relation}`. Serve `GET /apis/.../topology` snapshot plus deltas on the stream (`topologyDelta` frames, same seq/epoch). Edges to objects outside the caller's scope are dropped, never placeholders.
**Rationale**: Nearly free (already-held stores); gives the near-real-time topology.

## R12. Metrics ring (FR-019)

**Decision**: Sample metrics.k8s.io (node and pod CPU/memory) every 15 s when the API exists; ring of ~240 samples per series (~1 h), a fixed small series set per object, bounded total memory; explicit truncation marker when a query reaches past the ring; `metricsUnavailable` capability flag when the metrics API is absent. No external dependency.
**Alternatives**: scrape Prometheus (needs discovery/auth, out of scope); durable TSDB (R13).

## R13. Storage interface and backend candidates (FR-020; follow-on spec)

**Decision**: Define an internal `SeriesStore` trait (`append`, `query_range`, `truncate_marker`) with the in-memory ring as the only 036 implementation. A follow-on spec adds durable adapters. Candidate backends to evaluate there (licences and operational fit verified from source at that time, not assumed): GreptimeDB, InfluxDB (3.x), CnosDB, QuestDB, and a remote option (Prometheus remote-write / OTLP / operator-run backend). Rule: bundled or sidecar backends must be permissively licensed (Apache-2.0, MIT, BSD); remote adapters may be any licence.
**Rationale**: Keeps 036 shippable and avoids locking in a database before the licence and ops review; the trait makes the choice reversible.

## R14. T041 verification on kind (identity forwarding)

Verified 2026-09-24 on the local kind cluster (Kubernetes v1.32.2 node, kubectl v1.36.3) with an HTTP/HTTPS header-echo pod:

- **Plain service proxy** (`/api/v1/namespaces/<ns>/services/<svc>:80/proxy/`): the backend received only `x-forwarded-*` headers. No `X-Remote-User`, no `X-Remote-Group`. The caller's identity is NOT forwarded.
- **APIService** (`apiregistration.k8s.io/v1`, `insecureSkipTLSVerify` for the probe only), request via `/apis/<group>/v1/...`: the backend received `x-remote-user`, `x-remote-group` and `x-remote-extra-*` set by the aggregator, and the APIService reported `Available=True`.

Conclusion: per-user filtering must use the APIService path; the daemon must never take identity from anything but front-proxy headers on a mutually-verified connection. Not covered: the requestheader client certificate itself (the probe echo server did not request one), so client-cert verification against the requestheader CA is unit/handshake tested locally but not yet exercised through a real aggregator. The probe namespace and APIService were deleted afterwards.

## R15. Warm-snapshot benchmark and resource budget (T036, T037, T052)

Measured 2026-09-26 on a dedicated kind cluster (`fathom-daemon-bench`, Kubernetes v1.32.2, 1 real node plus 50 kwok nodes; fixture `k8s/bench-fixture.sh`). The cluster held 5,019 pods, 122 workloads (Deployment/ReplicaSet/Job/...), about 240 Events and 51 Nodes: roughly 5,400 watched objects. The host (32 threads) was shared with other agents' builds and had a load average of 27 to 45 during the client runs, so absolute times are noisy. Off and auto runs were interleaved to keep the comparison fair. Images were `fathom-daemon:bench036` and, after the render fix below, `bench036b`. The client was the real backend (`live_driver` example, release build).

**T036: time to first populated list.** First, the raw endpoint via `kubectl get --raw` (20 runs, milliseconds, includes kubectl start-up of about 30 to 40 ms):

| Path | min | p50 | max | Body |
|---|---|---|---|---|
| Direct `GET /api/v1/pods` (admin) | 199 | 216 | 296 | 21.2 MB |
| Direct `GET /api/v1/pods?resourceVersion=0` (watch cache) | 193 | 204 | 273 | 21.2 MB |
| Daemon `/v1/snapshot/pods` (admin) | 133 | 149 | 177 | 13.2 MB |
| Direct pods (user `carol`, cluster-wide `view`) | 194 | 203 | 275 | 21.2 MB |
| Daemon pods (`carol`, SubjectAccessReview-filtered) | 142 | 163 | 193 | 13.2 MB |
| Direct `GET /api/v1/events` | 43 | 46 | 52 | 257 kB |
| Daemon `/v1/snapshot/events` | 38 | 44 | 53 | 178 kB |

The snapshot body is about 38% smaller because the daemon strips `managedFields`.

Second, in the app (10 interleaved runs each, from `SetContext` to the first `PodsUpdated` carrying all 5,019 pods):

| Preference | Total p50 (min to max) | Pod stage p50 (min to max) |
|---|---|---|
| `off` (direct LIST) | 2,617 ms (1,510 to 3,564) | 1,311 ms (676 to 1,746) |
| `auto` (daemon snapshot) | 2,302 ms (1,180 to 2,765) | 810 ms (372 to 1,026) |

The pod stage runs from `NamespacesUpdated` to `PodsUpdated`. Every `auto` run was served from the snapshot (`age_ms=0` in the debug log). The daemon path was faster at min, p50 and max, so it was never worse than direct. About 1.1 to 1.3 s of the total happens before the pod list starts, in both modes (context start, then the namespace list), and the daemon does not change it. With the daemon uninstalled, `auto` probes `notInstalled` and falls back to the direct LIST, which returned the full list (3.7 s under load).

**T037: daemon budget.** RSS was read from `/proc` in the kind node; idle CPU is utime+stime over 60 to 120 s.

| State | RSS |
|---|---|
| Synced, no snapshot served yet | 146.4 MiB |
| After one pods snapshot | 146.6 MiB (high-water mark 156 MiB during the render) |
| After 20 concurrent pods snapshots | 146.9 MiB (high-water mark 179 MiB) |
| metrics-server installed, ring sampling for 5 min | 144.2 MiB |

Idle CPU was 0.05 to 0.09% of one core, against a target under 2%.

Before the fix, 10 concurrent pods snapshots OOM-killed the daemon at its 256Mi limit. Each render holds a full ~13 MiB body, and a doubling `Vec` briefly doubles that. Now at most `MAX_CONCURRENT_RENDERS` (2) renders run at once; more callers queue and are never rejected. The buffer is also sized up front from the previous render's bytes per item.

So steady-state RSS is under 150 MiB at about 5,400 objects, but only just, and the budget has two gaps:
- **Transient peaks exceed it** while snapshots are served.
- **The metrics ring was barely exercised.** kwok pods have no metrics, so only the ~14 real pods had series. At 5,000 real pods the ring grows toward its 32 MiB cap (`DEFAULT_MAX_BYTES`), which would put the daemon over budget. Meeting 150 MiB there needs a lower ring cap, or leaner stored objects (for example, projecting pods to the fields the snapshot and topology need).

**T037: client idle CPU with the stream open.** The same backend, idle for 120 s after the first list. The cluster was not silent: a CronJob, a failing Job and kwok status updates were running.

| Preference | Backend CPU | RSS | List re-emits in the window |
|---|---|---|---|
| `auto` (snapshot + one WATCH + daemon stream) | 3.2% of one core | 842 MB | 8 |
| `off` (legacy pod watcher, re-LISTs every event) | 64.9% of one core | 1.5 GB | 114 |

Both modes also include the add-on overview pollers (cert-manager, Flux and others).

**T052 (SC-007).** `k8s/test-daemon.sh` on this cluster, 2 full passes:
- Alice (`view` on `daemon-demo`) sees only `daemon-demo` topology: 14 to 18 nodes and 11 to 15 edges, every edge between her own nodes.
- Bob sees nothing.
- The admin sees 16 to 17 namespaces.
- Scaling `rollout-demo` reached Alice's stream as a `topologyDelta` in 50 ms and 430 ms (target 5 s or less), with no out-of-scope nodes.

These runs also exercised the front-proxy mTLS path through a real aggregator (see R14).
