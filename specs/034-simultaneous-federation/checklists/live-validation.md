# Live validation: 034-simultaneous-federation (T030, T031)

Run 2026-09-26 on Linux (Pop!_OS 24.04, kernel 7.1.5, 32 cores, Docker 29.8.1, kind v0.27.0).
There were 5 throwaway kind clusters, `fathom-fed-1` to `fathom-fed-5` (Kubernetes v1.32.2, one node each), and all were deleted afterwards.

**Harness.** The real backend ran headless via `examples/live_driver.rs`, the same `spawn_backend` the Tauri shell uses. It was driven with `BackendCmd` JSON lines, with isolated `HOME` (so `~/.fathom`) and `KUBECONFIG`. No WebKitWebDriver is available on this host, so the Tauri window itself was not automated. The UI rendering of these states is covered by the fake-backend Playwright suites (`us1`–`us3` federation specs).

## T031 (SC-004): reachability loss and recovery

The context `kind-fathom-fed-3` was federated with 4 others and all 5 were `Live`. Then `docker stop fathom-fed-3-control-plane` was run, followed by `docker start` about 45s later. `docker stop` itself takes about 11s (graceful stop), so times are measured from the first watch failure.

**Before the fix (`UNREACHABLE_AFTER` = 30s, `DEGRADED_TICK` = 5s):**
- `Reconnecting` came 10.8s after the stop command, as soon as the apiserver died. That state has no error text.
- `Unreachable` came at 41.2s, with error `failed to start watching object: ServiceError: client error (Connect)`. That is about 30s after the failure, so it **fails** SC-004 ("shows an error within 15s").

**Fix:** `src/backend/federation.rs` now has `UNREACHABLE_AFTER` = 10s and `DEGRADED_TICK` = 2s. A unit test pins `UNREACHABLE_AFTER + DEGRADED_TICK <= 15s`.

**After the fix (same procedure):**
- `Reconnecting` came at 10.8s after the stop command.
- `Unreachable` came at 20.9s, which is **10.1s after the failure**. The verbatim error is shown in the row. **Pass.**
- While fed-3 was down, fed-1 kept updating: a pod created there was reflected 0.85s later and fed-1 stayed `Live`.
- After `docker start`, the apiserver `/readyz` answered at 7.3s. fed-3 returned to `Live` 9.0s after `/readyz` (16.2s after start). This is within one watcher backoff cycle (kube-runtime default backoff, capped at 30s). No operator action was needed. **Pass.**

**Also observed during the same runs:**
- SC-001: a `nope/nope` pod in fed-2 reached the rollup as pending after 0.61s and as unhealthy after 2.3s. The combined unhealthy count rose too, and no other context changed. **Pass.**
- T027 restore: after a restart, `Initialize` restored the 5-context set from `~/.fathom/federation.json`, and all were `Live` within 1.2s. Unfederating and refederating fed-5 updated the persisted set.

## T030 (SC-002): idle CPU with 5 federated contexts

The backend restored the 5 federated contexts on `Initialize`. After 45s of settling it was sampled for 120s. The measure was utime+stime from `/proc/<pid>/stat` for the backend process, the same method as `scripts/measure-idle-cpu.sh`.

| build | CPU (% of one core) | RSS | threads |
|-------|---------------------|-----|---------|
| debug (unoptimised) | **0.092%** (11 ticks in 120s) | 28.5 MiB | 35 |

**Load conditions.** Other agents were compiling at the same time. The loadavg (1/5/15 min) was 26.1/39.6/42.0 at the window start and 14.9/31.4/38.8 at the end, on 32 cores. Contention can only inflate the wall-clock-normalised figure through scheduling noise, not lower it. The debug build is slower than release, so this is an upper bound for the backend.

**Result: pass for the backend, which is well under the 2% budget.** Not measured: the WebKitGTK webview's own idle cost, since the Tauri window could not be driven headless on this host. The UI half of SC-002 (the 100ms input target) is also not measured here.
