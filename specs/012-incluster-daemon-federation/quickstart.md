# Quickstart: Validating P1 (Multi-Context Switcher & Live Status)

## Prerequisites

- Rust toolchain + `cargo`, Node/`pnpm` set up as usual for this repo (`ui/`).
- For a manual/live check (optional — automated tests below don't need a real cluster): a kubeconfig with
  at least 2 contexts, one pointed at an unreachable API server (e.g. a stale IP) to exercise the `Error`
  status.

## Automated validation (what CI/this task's verification runs)

```bash
# Backend: protocol round-trip + status-derivation/probe-skip unit tests
cargo build
cargo clippy --all-targets -- -D warnings
cargo test

# Frontend: typecheck, lint, unit tests, e2e
pnpm -C ui typecheck
pnpm -C ui lint
pnpm -C ui test          # Vitest, includes the new system.federation.test.ts
pnpm -C ui test:e2e -- us1-federation-context-status   # or the project's e2e runner equivalent
```

All of the above MUST be clean before this phase is considered done (per the task's explicit testing
requirements).

## Manual scenario walkthrough (maps to spec.md's Acceptance Scenarios)

1. Launch Fathom against a kubeconfig with 3 contexts, one unreachable.
2. **Expect**: all 3 contexts appear immediately in the new always-visible strip under the top bar
   (`federation-context-switcher`) — no click needed to reveal them (FR-001).
3. **Expect**: within a few seconds, each shows a status dot — the active one `connected`, the reachable
   inactive one `not connected`, the unreachable one `error` (FR-002).
4. Hover/click the `error` context. **Expect**: the underlying error text is visible without switching to
   it (FR-004).
5. Click a different, reachable context. **Expect**: it becomes active (`SetContext` sent, same reconnect
   behavior as today), its dot turns `connected`, the previously-active one turns `not connected`, and
   resource views refresh (FR-003).
6. Edit the kubeconfig on disk (add/remove a context) and trigger Fathom's existing reload path.
   **Expect**: the strip updates without an app restart (FR-005).

## Out of scope for this quickstart

P2 (simultaneous federation), P3 (cloud auth-plugin shellouts), P4 (optional in-cluster daemon) — see
spec.md; not designed or built in this pass.
