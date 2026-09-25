# Quickstart: Local Mutation Audit-Log Viewer

## Prerequisites

- A working Fathom dev build (`cargo build`, `pnpm -C ui install`) connected to any reachable
  cluster (a kind/minikube/dev-sandbox cluster is sufficient — no special fixtures needed beyond
  what other mutation-related specs already use, e.g. `dev/sandbox/`).
- No new external services or credentials required; this feature only adds a local file
  (`~/.fathom/audit-log.json`) and a UI pane.

## Manual validation (User Story 1 — P1)

1. Launch Fathom against a dev cluster.
2. Perform any mutating action, e.g. cordon a node (Nodes view → select a node → Cordon).
3. Open the new **Audit Log** pane (new nav entry / panel, per implementation).
4. Confirm the cordon action appears at the top of the list with: action type ("Node Cordon"),
   target resource name, cluster context, a recent timestamp, and outcome = success.
5. Repeat with an action expected to fail (e.g. attempt a Helm upgrade with an invalid values
   file) and confirm it appears with outcome = failure and a failure reason.

**Expected outcome**: both entries visible, correctly attributed, within ~2 seconds of the
action completing (SC-001).

## Manual validation (User Story 2 — P2)

1. With at least two different action types and at least one failure in the history (from above),
   open the Audit Log pane.
2. Apply a filter for action type = "Node Cordon" → confirm only cordon entries remain.
3. Clear the filter, then filter by outcome = "Failed" → confirm only the failed entry(ies) remain.
4. Clear the filter, then type part of a target resource's name into the search box → confirm only
   matching entries remain, case-insensitively.
5. Clear all filters/search → confirm the full history reappears.

## Manual validation (User Story 3 — P3, retention)

1. Perform mutating actions until the count exceeds the fixed retention bound (500 — see
   `research.md` R3), or (for practical dev testing) temporarily lower the bound via a debug
   build constant and repeat.
2. Confirm the oldest entries beyond the bound are no longer present in the viewer.
3. Confirm no entry in the viewer corresponds to a cluster-side (Kubernetes API server) audit
   event that Fathom itself did not initiate.

## Automated validation

- Rust: `cargo test` covers the new `audit_summary()` classification (exhaustive match,
  unit-tested per variant category) and the `audit-log.json` load/save/retention-truncation
  round trip (`src/backend/persistence.rs` or a new `src/backend/audit_log.rs`).
- Frontend/E2E: `ui/tests/e2e/audit-log.spec.ts` (Playwright) drives the UI against the scripted
  fake backend (per Constitution Principle III): performs one mutating action (e.g. node cordon),
  opens the Audit Log viewer, asserts the entry's target/outcome/timestamp are correct, then
  applies one filter/search and asserts the result narrows correctly.
- Full gate: `cargo build --workspace --all-targets`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test`, plus the new
  Playwright spec, must all pass before merge (Constitution Principle III).

## Explicit non-goals reminder

Do not implement: cluster-side (Kubernetes API server) audit log reading, log export/sharing, or
a retention-policy settings UI. See `spec.md` for the full non-goals list.
