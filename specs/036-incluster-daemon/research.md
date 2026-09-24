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
