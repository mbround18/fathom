# Research: Simultaneous Federation

**R1 Extend vs. duplicate 027.** Decision: extract client construction, the unhealthy-pod classifier, the concurrency cap and staggered start from `src/backend/fleet.rs` into shared helpers, and add `src/backend/federation.rs` for the persistent-client + watch tier. Rejected: copying the poller (drift risk on classifier and RBAC handling).

**R2 Live mechanism.** Decision: kube-rs `watcher` (`kube::runtime`) on Pods, Nodes, Deployments and Events per context, reduced into in-memory counters (no object stores) to bound memory. RBAC-denied watches mark that metric "no access". Rejected: shorter-interval polling (not live, multiplies API load). Verify the `kube` `runtime` feature is enabled; it is a feature flag, not a new crate.

**R3 Reuse of the 027 poller.** Decision: a context lives in exactly one tier, polled (027, dialog-open only) or live (federated). A tier arbiter in `FleetState` stops the poller when federation starts and resumes it (if the dialog is open) on removal. Federated summaries are also emitted as `FleetContextSummary` via `FleetSummaryUpdated` plus an additive `live` marker, so 027 UI keeps working.

**R4 Persistence.** Decision: `~/.fathom/federation.json` `{version:1, contexts:[...], softCap:8}`, additive, atomic write via existing `persistence.rs` helpers, empty fallback on parse error without deleting the file (Principle VI).

**R5 Backoff.** Decision: exponential 2s to 60s with jitter per context; watch errors trigger relist. Rejected: fixed retry (thundering herd after a VPN drop).

**R6 Event volume.** Decision: per-context 500ms debounce coalescing into one summary event; combined rollup recomputed in the backend and emitted on change only. Idle cost is watch keepalives only.

**R7 Staleness.** Decision: monotonic per-context `generation: u64` on every emitted event; the store drops lower generations (Principle V). Re-federating bumps it.

**R8 Protocol.** Decision: additive commands `FederateContext`, `UnfederateContext`, `RequestFederationSnapshot`; additive events `FederationStateUpdated`, `FederationRollupUpdated`. Additive, so no protocol version bump; round-trip and ts-rs drift checks extended.

**R9 UI surface.** Decision: a "Federation" section inside the existing Fleet dialog (no new nav section), per-context federate toggles, aggregate header plus per-cluster rows, mobile-first single column.

**R10 Risks.** Watch fan-out on very large clusters (counters only, cap 8); token/exec-plugin expiry (row-level auth error); Constitution V pressure (Complexity Tracking); refactoring 027 code risks regressing its tests (keep 027 suite green as a gate).

**Implementation notes (034-impl).** R2: the `kube` `runtime` feature was already enabled in `Cargo.toml`; no change. R1 deviation: to keep the parallel 035 work conflict-free, `fleet.rs` gained only additive shared helpers (`client_for_context`, `stop_poller`, and widened visibility of the classifier, `is_auth_error`, `start_offset`, `emit_summary`); `poll_inner` was not rewritten to call them. R5: backoff is 1s base doubling to a 60s cap with equal jitter (half fixed, half random). Not yet run: T030 idle-CPU soak (SC-002) and T031 kind stop/start recovery (SC-004); no result is recorded here because none was measured.
