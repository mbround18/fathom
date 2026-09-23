# Quickstart: Pod Lifecycle Diagnostics

## Prerequisites

- Rust toolchain + `cargo` (backend).
- `pnpm` + Node (frontend), `ui/` dependencies installed (`pnpm -C ui install`).
- No live cluster required for validation — all tests run against fixtures/fake backend.

## Backend validation

```bash
cargo build
cargo clippy --all-targets -- -D warnings
cargo test                              # includes protocol_serde round-trip + pods.rs unit tests
cargo test --test export_bindings       # regenerates ui/src/protocol/generated/*.ts; must be a no-op diff if already committed
```

Expected: all green. `src/backend/pods.rs`'s new `#[cfg(test)] mod tests` cover:
- an init container that succeeded (exit 0) vs. one that failed (non-zero exit), FR-002/FR-003
- a container with no probes configured vs. one with liveness+readiness+startup all configured
- a container with `restart_count == 0` (no `last_state`) vs. one with a prior `OOMKilled`
  termination, FR-008/FR-009
- a container with `postStart`/`preStop` hooks configured vs. none, FR-010/FR-012
- a `restartPolicy: Never` pod where `last_state` is absent but `state` is terminated (edge case)

## Frontend validation

```bash
pnpm -C ui typecheck
pnpm -C ui lint
pnpm -C ui test                         # Vitest: podLifecycle.ts correlation unit tests
pnpm -C ui test:e2e -- us1-pod-lifecycle-init-containers us2-pod-lifecycle-probes \
  us3-pod-lifecycle-restarts us4-pod-lifecycle-hooks us5-pod-lifecycle-events-timeline
```

(Check `ui/package.json` scripts for exact e2e invocation syntax before running; mirror whatever
006-helm-mutate's e2e specs use.)

## Manual end-to-end scenario (matches spec.md's Independent Tests)

1. Deploy a Pod with two init containers (one exits 0, one exits 1) and one app container that
   never starts. Open it in Fathom's Pod Inspector → confirm a distinct "Init Containers" section
   shows both, the failing one with its exit code, and its logs reachable via the Logs tab
   (US1).
2. Deploy a Pod with a deliberately-misconfigured `readinessProbe`. Open its inspector → confirm
   the probe's config (type/thresholds/period) is shown and flagged as currently failing (US2).
3. Deploy a Pod with a container that crashes and restarts (`command: ["sh","-c","exit 1"]`).
   Open its inspector → confirm that container's restart count, last reason, exit code, and
   last-terminated timestamp are shown, distinct from a healthy sibling container (US3).
4. Deploy a container with a failing `postStart` hook. Open its inspector → confirm the hook's
   config is shown and a hook-failure indication (from a `FailedPostStartHook` Event) appears
   attached to that container (US4).
5. Open any Pod with several Events. Confirm they appear as a chronological timeline inside the
   inspector without navigating to the cluster-wide Events view, and that it live-updates while
   open (US5).

## Rollback

This feature only adds a field to `PodDetails` and new UI sections; reverting the branch is
sufficient (no migrations, no persisted state, no protocol version bump).
