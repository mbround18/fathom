# Quickstart: validating 055 (Keel)

These steps are run by the main session, one at a time, with `CARGO_BUILD_JOBS=4`. Subagents run
only the targeted commands in §1.

Database tests use a throwaway PostgreSQL. Point `KEEL_TEST_DATABASE_URL` at it; each test creates
and drops its own schema. If the variable is unset, the tests are skipped with a note, never failed.

## 1. Targeted tests (no Docker)

```
cargo test -p keel-core        # policy resolution, run state machine, approval binding to (commit, inventory rev),
                               # TTL/four-eyes, rollback availability, inventory validation, import planning (no I/O)
cargo test -p keel-protocol    # round trips for every Cmd/Event; ts-rs bindings drift check
cargo test -p keel-git         # gix adapter against local bare repos: fetch, pins, force-push detection
cargo test -p keel-exec        # frame round trips; fake runner session
cargo test -p web-passkeys     # soft passkey, as moved from Depths
cargo test -p depths-server --test passkeys   # Depths still passes after the extraction
cargo test -p keel-server      # with KEEL_TEST_DATABASE_URL: receivers, sync, queue, audit chain, export/import
pnpm --filter @companion/keel-web test
pnpm --filter @companion/keel-web exec playwright test --project <us1..us6>   # fake backend, one project at a time
```

## 2. Scenarios, per story

Each scenario runs against `keel-server` with a fake runner (`keel-exec` test double) and local
bare repositories. Shapes are in the contracts; this list states only what has to be true.

| Story | Scenario | Expected |
|---|---|---|
| US1 bind | Create a binding and a GitHub-style receiver, then post a signed push | `accepted`, then `synced`; jobs read from `.keel/jobs.yaml`. A bad signature gives `401`, a replayed delivery `duplicate`, an unbound repository `403`, all audited (SC-002). |
| US1 bind | Rotate the receiver secret | The old secret works until `previous_expires_at`, then gives `401`. |
| US2 drive | A push touches a job's `paths` | A run is created, pinned to the fetched commit (not the payload's), and goes through policy. The fake runner's events show per-host results within 30 s of finishing (SC-003). |
| US2 drive | Force-push the branch | `history_rewritten`; no run unless the binding allows force-push. |
| US3 govern | Policy requires approval; the author tries to approve | Refused (four-eyes). Another approver approves with step-up; the run starts. A new push before approval leaves the old approval unable to start a different commit. |
| US3 govern | Run `keel audit verify` after the scenarios | The chain verifies; every run action is present with actor and time (SC-004). Editing an `audit_event` row as the app role fails. |
| US4 inventory | Edit a managed inventory from a stale base | `stale_base`. From the head, a new revision; a running run keeps its pinned revision. Invalid vars against the schema are refused before saving. |
| US5 rollback | Roll back a run whose job has `rollback.playbook` | A new run, linked to the original, through the same policy. Without rollback config: `rollback_unavailable`. |
| US5 migration | Export a workspace, import into a second instance's schema | Dry run lists conflicts. With `rename`, the import applies in one transaction, references resolve, and no credential values cross (SC-007). A tampered bundle is refused before anything is written. |
| Runners | Enrol a runner with a token, then connect with a different server certificate; connect an unenrolled client certificate; reuse the token; revoke a live runner | Pinned fingerprint mismatch: the runner refuses. Unknown certificate: `unknown_runner`. Reused token: refused and audited. Revoke: the connection closes and its run becomes `failed (runner_lost)`. |
| US6 teams | An operator in team A acts on team B's workspace | `Denied`, audited (SC-006). Removing a user's grant closes their open socket (`Revoked`). |
| Standalone | Run the full `keel-server` suite with no suite settings | All pass (SC-008). |

## 3. Real ansible (opt-in)

```
docker build -f apps/keel/Dockerfile --target runner -t keel-runner:dev .
KEEL_IT_ANSIBLE=1 cargo test -p keel-runner --test ansible
```

The test runs a fixture playbook against `localhost` (connection `local`) inside the runner
container, then checks that:

- `keel_events` lines arrive in order and per-host stats match `PLAY RECAP`;
- a playbook that leaves a background process behind makes the runner report `Tainted`;
- secrets files are gone from `/work` after `Cleaned`.

It removes the containers it started, by name.

## 4. End to end on the dev compose

1. `keel keygen > master.key` (mode `0400`), copy `.env.example` to `.env`, and run
   `docker compose -f apps/keel/compose.yaml up -d`.
2. Sign in as the owner, register a passkey, and enrol a runner with the shown token.
3. Bind a repository on git.boop.ninja (Forgejo receiver) that contains `.keel/jobs.yaml` and a
   playbook against a throwaway container host.
4. Push. The delivery shows `accepted`, the run waits for approval, the approval needs a passkey,
   and the log streams live.
5. Stop `keel-runner` mid-run. The run becomes `failed (runner_lost)` after about 45 s.
6. Run `docker compose -f apps/keel/compose.yaml down`. Volumes are left in place; remove them
   only by name, by hand.
