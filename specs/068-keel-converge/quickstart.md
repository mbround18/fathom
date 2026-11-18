# Quickstart: verifying spec 068

All commands from the repo root, one at a time, `CARGO_BUILD_JOBS=4`, logs to files. Server
tests need `KEEL_TEST_DATABASE_URL` (`scripts/keel-test-db.sh`); report each file's
`running N tests` line.

## 1. Per-story checks (fail before, pass after)

| Story | Command | Proves |
|---|---|---|
| Core | `cargo test -p keel-core converge` | expand order, first-position dedupe, cycle named, coverage closure, `host_plan` drops covered steps, `combine_policies`, `suggest` |
| Core | `cargo test -p keel-core manifest` | new keys parse; every new error code; old manifests unchanged (SC-006) |
| US1 | `cargo test -p keel-server --test converge` | plan → approve once → steps in order; failure skips rest on that host only; retry resumes at failed step |
| US1 | `cargo test -p keel-server --test converge_lock` | a path run created between two steps waits until the chain ends; two holders impossible; lost step releases hold; restart reconcile |
| US2 | `cargo test -p keel-server --test converge enroll` | enroll → converge as `system:enroll`; checkbox off ⇒ none; flag off ⇒ none |
| US3 | `cargo test -p keel-server --test host_profiles` | step-up required; cert principals updated; suggestions from `groups:`; never auto-assigned |
| US4 | `cargo test -p keel-server --test coverage` | narrow path run ends `covered` on hosts with a covering converge at same/newer commit; queues elsewhere |
| US5 | `cargo test -p keel-trust ledger` and `cargo test -p keel-server --test check_mode` and `cargo test -p keel-runner --test check_mode` | `--check --diff`; no `host_revisions`, drift or rollback point; ledger `Checked` doesn't move base (SC-004) |
| US6 | `cargo test -p keel-server --test env_canary` and `cargo test -p keel-runner --test env_vars` | canary value absent from logs, events, audit, all columns (SC-005); reserved names refused |
| US7 | `cargo test -p keel-forge statuses` and `cargo test -p keel-server --test required_checks` | waiting → queued on success; `checks_failed`; timeout; unsupported forge refused at preview |
| US8 | `cargo test -p keel-server --test choice_params` and `cargo test -p keel-runner --test choice` and `--test prompt` | `from:` listing at commit; runner re-check; a `pause` play fails in < 5 s |
| Web | `pnpm --filter @companion/keel-web test` and `pnpm --filter @companion/keel-web exec playwright test --project=s068` | |

## 2. End to end on the dev stack (SC-001)

1. `make -C apps/keel compose-up` with `KEEL_CONVERGE=1` in `apps/keel/.env`, plus a dev runner container (as in 066 quickstart).
2. Bind a fixture repo (`apps/keel/server/tests/fixtures/converge-repo`) whose manifest has
   `keel-node: {includes: [debian], jobs: [keel-node]}` and fixture playbooks that write a marker
   file per job.
3. Create an enrollment with profiles `debian, keel-node`, auto-converge on; enroll the dev
   runner container (its ceiling includes `debian`).
4. Expect: one converge, one approval, steps `debian` then `keel-node`, both markers present,
   applied revision set for both jobs, no manual run.

## 3. Check mode (SC-004)

Start the same converge with Check on. Expect `check` badges, `--check --diff` in the log, the
markers absent, `host_revisions` unchanged, ledger tail `Checked`.

## 4. Rolling the manifest out to a real fleet (research R4)

1. Deploy server 068 and bump `keel_runner_sha256` to the 068 runner in `../ansible`; let the
   `keel-node` job upgrade every runner.
2. Binding page: "0 runners would refuse this manifest".
3. Land contracts/ansible.md §5 (ceilings), run `keel-node` once more so policies pick it up.
4. Land §1–§4, §6–§7. Sync; check suggestions on Hosts; accept them; converge one host first.

## 5. Not covered here

The full gate (`scripts/ci.sh`) runs once on the combined branch, by the main session.
