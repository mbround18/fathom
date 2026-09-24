# Tasks: Optional In-Cluster Daemon

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)
**Tests**: Required (Constitution III). Write each test first and confirm it fails.
**Format**: `[ID] [P?] [Story] Description` — [P] = parallelizable (different files).
Tasks marked (needs 031) depend on `k8s/` and `deploy/Dockerfile` from spec 031 being merged.

## Phase 1: Setup

- [X] T001 Add workspace member `daemon/` (`daemon/Cargo.toml`, `daemon/src/main.rs` stub) and register in root `Cargo.toml`
- [X] T002 [P] Add `axum` 0.8 (align with 031) and dev-dep `tower-test` to `daemon/Cargo.toml`
- [X] T003 Verify `cargo tree -p fathom-daemon` has no egui/eframe/tauri; if the lib leaks UI deps, gate them behind the `egui-ui` feature or extract shared modules (research R3)

## Phase 2: Foundational (blocks all stories)

- [X] T004 Create `src/protocol/v1/daemon.rs` with all types from data-model.md; export in `src/protocol/v1/mod.rs`
- [X] T005 [P] Add commands `SetDaemonPreference`, `RefreshDaemon`, `RequestDaemonTimeline` in `src/protocol/v1/cmd.rs` and events `DaemonStatusUpdated`, `DaemonEventsBatch` in `src/protocol/v1/event.rs`
- [X] T006 [P] Round-trip tests for every new variant/type in `src/protocol/v1/event.rs`/`cmd.rs`/`daemon.rs` test modules
- [X] T007 Regenerate TS bindings into `ui/src/protocol/generated/`; confirm drift check passes
- [X] T008 [P] Additive `daemon.json` load/save (preference + cursor) in `src/config.rs` with default-fallback-without-deleting test (Principle VI)

**Checkpoint**: shared contract merged; Phases 3-5 can proceed in parallel on separate agents.

## Phase 3: US1 daemon side — replayable controller events (P1)

- [ ] T009 [P] [US1] Tests: `EventLog` append/eviction by count and age, contiguous seq, cursor replay, epoch mismatch and too-old cursor yield `truncated` — `daemon/tests/replay_tests.rs`
- [ ] T010 [US1] Implement `EventLog` + bounded broadcast fan-out with slow-subscriber drop — `daemon/src/eventlog.rs`
- [ ] T011 [P] [US1] Tests: derivation table (Deployment rollout/ProgressDeadlineExceeded, StatefulSet/DaemonSet, Job Complete/Failed, CronJob, HPA scaling, Pod CrashLoop/OOMKilled/Ready) and coalescing with `repeat` — `daemon/tests/derive_tests.rs`
- [ ] T012 [US1] Implement status-diff derivation and coalescing, messages verbatim and capped — `daemon/src/derive.rs`
- [ ] T013 [US1] Watchers via `kube::runtime` reflectors for kinds in FR-003 plus raw Events forwarding, namespace allowlist support — `daemon/src/watchers.rs`
- [ ] T014 [P] [US1] Tests (mock kube + axum test client): `/healthz`, `/readyz`, `/v1/capabilities`, `/v1/stream` resume/truncation/heartbeat, 429 over stream cap, JSON errors — `daemon/tests/http_tests.rs`
- [ ] T041 [P] [US1] Tests first: verify on kind that the plain service proxy forwards no identity and that an `APIService` delivers verified `X-Remote-User`/groups (research R1); record the result in research.md — `k8s/test-daemon.sh`
- [ ] T042 [P] [US1] Tests: identity parsing only from verified front-proxy client cert (reject unverified/spoofed headers, query params); missing identity yields empty scope — `daemon/tests/identity_tests.rs`
- [ ] T043 [P] [US1] Tests: `VisibleScope` via mocked `SubjectAccessReview` (two users, different namespaces, 60 s cache, error fails closed, revoked access emits `scopeChanged`, cluster-scoped kinds need cluster list) — `daemon/tests/authz_tests.rs`
- [ ] T044 [US1] Implement `identity.rs` and `authz.rs` (SAR batching, bounded cache) and apply the filter to replay and live frames before any endpoint returns data — `daemon/src/{identity.rs,authz.rs}`
- [ ] T015 [US1] Implement HTTP surface and `main.rs` config from env (DaemonConfig), graceful shutdown, secret-free logging — `daemon/src/{http.rs,config.rs,main.rs}`

## Phase 4: US1 + US2 client — timeline and graceful absence (P1)

- [X] T016 [P] [US2] Tests: probe classification (404/503 notInstalled, 403 noAccess, timeout unavailable, major mismatch), zero requests when preference `off`, probe timeout 5 s, backoff 5 s to 5 min — `src/backend/daemon_client.rs` test module
- [X] T017 [US2] Implement probe + backoff via `kube` client raw requests to `/apis/daemon.fathom.dev/v1` (never sends tokens to the daemon); emit `DaemonStatusUpdated` — `src/backend/daemon_client.rs`
- [X] T018 [US1] Implement stream consumer with cursor resume, batching (200 / 250 ms), session tagging and stale-session discard, truncation handling — `src/backend/daemon_client.rs`
- [X] T019 [US2] Hook into connect/refresh/context-switch paths without blocking connection; verify existing behavior unchanged with feature off — `src/backend/kube.rs`, `src/backend/cmd.rs`
- [X] T020 [US2] Command handlers for `SetDaemonPreference`/`RefreshDaemon`/`RequestDaemonTimeline` persisting via T008 — `src/backend/cmd/`
- [ ] T021 [P] [US1] UI store slice and timeline merge/dedupe/staleness Vitest tests — `ui/src/store/`, `ui/src/**/__tests__/`
- [ ] T022 [US1] UI: "While you were away" timeline pane (derived vs raw badge, verbatim reason/message, truncation banner, responsive per Principle IV) — `ui/src/`
- [ ] T023 [P] [US2] UI: per-context daemon status chip with inspectable detail, "Use in-cluster daemon if present" toggle, install-command hint (copy only, no action) — `ui/src/`
- [ ] T024 [US1] Playwright vs scripted fake backend covering all six states, timeline, truncation banner — `ui/tests/`
- [ ] T025 [US2] Regression test: existing e2e suite passes unchanged with preference `off`; add explicit assertion that no daemon commands/requests are emitted — `ui/tests/`, backend test

## Phase 5: US3 — packaging and kind (P2)

- [ ] T026 [US3] (needs 031) Add `fathom-daemon` binary/stage to `deploy/Dockerfile` (distroless static, non-root, read-only rootfs)
- [ ] T027 [P] [US3] (needs 031) `k8s/addons/fathom-daemon/` kustomization: namespace, ServiceAccount, read-only ClusterRole (no secrets/configmaps, no write verbs; plus `create` on `subjectaccessreviews` only), `APIService` `v1.daemon.fathom.dev`, auth-configmap read in `kube-system`, binding, single-replica Deployment with probes/limits/securityContext, ClusterIP Service, common labels `app.kubernetes.io/name=fathom-daemon`
- [ ] T028 [P] [US3] (needs 031) `k8s/overlays/daemon/kustomization.yaml` and optional namespaced-scope variant (Roles + `FATHOM_DAEMON_NAMESPACES`)
- [ ] T029 [P] [US3] (needs 031) `k8s/samples/daemon-events.yaml`: Deployment with controllable rollout, failing Job, CronJob, HPA target
- [ ] T030 [US3] (needs 031) `k8s/cluster.sh up-daemon` / `down-daemon`: build image, `kind load docker-image`, apply overlay and samples; document in `k8s/README.md`
- [ ] T031 [US3] (needs 031) `k8s/test-daemon.sh`: asserts Ready, capabilities, replay after simulated disconnect, truncation after rollout restart, and uninstall leaves no labeled objects
- [ ] T032 [P] [US3] Static checks: `kubectl kustomize k8s/overlays/daemon | kubeconform`, and a test asserting the rendered ClusterRole contains only get/list/watch, `create` on `subjectaccessreviews`, and no secrets, and that the uninstall selector covers the `APIService`

## Phase 6: US4 — warm snapshots (P3)

- [ ] T033 [P] [US4] Tests: snapshot endpoint (synced/unsynced, age), client staleness threshold 30 s and fallback — `daemon/tests/http_tests.rs`, `src/backend/daemon_client.rs`
- [ ] T034 [US4] Daemon `/v1/snapshot/{pods,events}` from reflector stores — `daemon/src/snapshot.rs`
- [ ] T035 [US4] Client snapshot-then-delta source with transparent fallback to direct LIST+WATCH — `src/backend/daemon_client.rs`, `src/backend/pods.rs`, `src/backend/events.rs`
- [ ] T036 [US4] Benchmark time-to-first-populated-list with/without daemon on a 5,000-pod kind fixture; record in research.md (never worse than direct)

## Phase 6b: US5 — near-real-time topology and short metrics history (P3)

- [ ] T045 [P] [US5] Tests: topology derivation (owner refs, selectors, pod-to-node, HPA target), deltas with shared seq, per-user edge dropping (no placeholders) — `daemon/tests/topology_tests.rs`
- [ ] T046 [US5] Implement `topology.rs` from existing reflectors plus Nodes and Services; `/v1/topology` and `topologyDelta` frames — `daemon/src/topology.rs`, `daemon/src/http.rs`
- [ ] T047 [P] [US5] Tests: `SeriesStore` trait contract, ring bound and truncation marker, `metricsUnavailable` when metrics API absent — `daemon/tests/metrics_tests.rs`
- [ ] T048 [US5] Implement `series_store.rs` (trait) and `metrics_ring.rs` (15 s sampling of metrics.k8s.io, ~1 h, bounded bytes); `/v1/metrics` filtered per caller — `daemon/src/`
- [X] T049 [P] [US5] Protocol: `RequestDaemonTopology`, `RequestDaemonMetrics`, `DaemonTopologyUpdated`, `DaemonMetricsUpdated`, new frames/types with round-trip tests; regenerate bindings — `src/protocol/v1/`
- [X] T050 [US5] Client: topology/metrics stream consumption with session tagging and batching — `src/backend/daemon_client.rs`
- [ ] T051 [US5] UI: topology pane fed by the daemon (falls back to the existing client-built topology when off/absent) and per-object trend sparkline with truncation marker; Playwright vs fake backend at all widths — `ui/src/`, `ui/tests/`
- [ ] T052 [US5] kind test with two users: only in-scope nodes/edges visible; topology change in <= 5 s (SC-007) — `k8s/test-daemon.sh`

## Phase 7: Polish

- [ ] T037 [P] Perf/budget check (150 MiB incl. topology and ring): daemon RSS and idle CPU at 5,000 objects (SC-004); client idle CPU with stream open
- [ ] T038 [P] Update `docs/MVP.md` §14 Tier D P4 status and add a short section to `k8s/README.md`
- [ ] T039 Run `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, UI lint/typecheck/Vitest/Playwright; execute quickstart end to end
- [ ] T040 Marketing-PDF screenshots of the timeline and status chip (final step, per project convention)

## Dependencies

- Phase 1 -> Phase 2 -> {Phase 3, Phase 4, Phase 5} in parallel -> {Phase 6, Phase 6b} -> Phase 7.
- T044 (per-user filter) precedes T015 and T046/T048: no endpoint may ship unfiltered.
- T010 before T013/T015; T012 before T013; T017 before T018-T020; T022/T023 after T021 and T007.
- Phase 5 tasks need 031 merged (or 031's `k8s/` files cherry-picked).
- MVP scope: Phases 1-4 (US1 + US2, including per-user filtering); US3 makes it deployable; US4 and US5 optional.
