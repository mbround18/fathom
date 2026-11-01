# Quickstart: validating Keel run approval

## Automated

From the repository root, run one command at a time:

```sh
CARGO_BUILD_JOBS=4 cargo test -p keel-core approval
CARGO_BUILD_JOBS=4 cargo test -p keel-server --test run_approval
cd apps/keel/web
pnpm run -s typecheck && pnpm exec vitest run
KEEL_E2E_PORT=4264 PLAYWRIGHT_WORKERS=2 pnpm exec playwright test --project=s064
```

The batch-end gate stays `scripts/ci.sh`, run once by the main session.

## Manual walk-through (closes 063 T053 / SC-001)

1. Sign in as the owner. Bind a repository, enrol a host, and create a receiver, following the 063
   empty states. Leave the catch-all policy as it is: one approval.
2. Push a commit. The run appears in Runs as "Waiting · 0 of 1", and the Runs nav shows **1**.
3. Open the run. The panel shows the job, the hosts, the commit, "Needs 1 approval" and when the
   window closes.
4. Select **Approve**. The preview names what will happen. Confirm, and complete the step-up.
5. Expected:
   - the run is queued, then runs;
   - the nav count disappears;
   - Administration → Audit shows `run_approved` and `run_approval_queued`.
6. Change the catch-all to two approvals with four-eyes, then start a run by hand. Approve it
   yourself: Keel refuses ("You started this run…"), and the audit log shows
   `run_approval_refused`.
7. Deny it with a reason. Expected: the run reads "Approval denied" with the reason, and no host
   receives it.
8. Set a policy with a 2-minute window and push. Wait 3 minutes. Expected: the run reads "Approval
   expired", and the audit log shows `run_approval_expired` by Keel.
