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
cargo test -p keel-exec        # frame round trips; fake runner session; SSH listener refuses shell/exec/forwarding
cargo test -p keel-trust       # signed-mode and override test vectors (contracts/signed-mode.md), ledger chain
cargo test -p web-passkeys     # soft passkey, as moved from Depths; verify_assertion vectors
cargo test -p depths-server --test passkeys   # Depths still passes after the extraction
cargo test -p keel-server      # with KEEL_TEST_DATABASE_URL: receivers, sync, queue, audit chain, export/import
pnpm --filter @companion/keel-web test
pnpm --filter @companion/keel-web exec playwright test --project <us1..us9>   # fake backend, one project at a time
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
| US7 hosts | Enrol a host with a token, then point it at a server whose host certificate is from another CA; connect with an unenrolled key; reuse the token; try `exec` and port forwarding; revoke a live host | Wrong CA: the runner refuses before sending. Unknown key: only enrollment is offered. Reused token: refused and audited. Exec and forwarding: refused. Revoke: the session closes, the serial is in the KRL, and a running host becomes `lost`. |
| US7 hosts | Assign a profile outside the host's `allowed_profiles` | Refused by Keel; a forged `Assign` for that profile is refused by the executor (`profile_not_allowed`). |
| US8 signed | Signed mode: push a signed commit; then an unsigned commit; then an unsigned commit under a signed merge | Signed: runs. Unsigned and laundered-by-merge: `needs_review`, with the failing commits listed. |
| US8 signed | Approve the review with a passkey, then replay the same approvals to another host, to the same host for a new run, and with the target changed | First: runs. Others: `bad_override` or `replay`, refused by the host, not by Keel. |
| US8 signed | Hostile server: a test server with the right CA sends an unsigned pack with Keel-signed but passkey-less approvals, a wrong `base`, and an older target | Every case refused by the executor (`unsigned`, `base_mismatch`, `downgrade`) (SC-009). |
| US9 profiles | An operator scoped to `gameserver` starts a run on a `debian`-only host, and approves a review whose hosts span both | `Denied`, audited. Host page shows per-job applied commit, authorization and drift within 30 s of a run (SC-011). |
| US6 teams | An operator in team A acts on team B's workspace | `Denied`, audited (SC-006). Removing a user's grant closes their open socket (`Revoked`). |
| Standalone | Run the full `keel-server` suite with no suite settings | All pass (SC-008). |

## 3. Real host runner (opt-in)

```
KEEL_IT_HOST=1 cargo test -p keel-runner --test host
```

The test starts a throwaway Debian container with systemd (named `keel-it-host-<pid>`), applies
the `keel_runner` role contract with a test token, and checks that:

- the runner enrols, and `keel-runner` holds no sudo and no `keel-exec` or `automation` access;
- a signed fixture job runs as `keel-exec` with `connection: local`, and `keel_events` lines
  arrive in order, with per-host stats matching `PLAY RECAP`;
- a job that leaves a background process behind leaves nothing once the executor unit ends;
- secrets files are gone after `Finished`, and `secret_vars` containing `{{ lookup('pipe', 'id') }}`
  are passed through untemplated;
- the ledger verifies, and `Hello` reports the new base.

It removes the container it started, by name.

## 4. End to end on the dev compose

1. `keel keygen > master.key` (mode `0400`), copy `.env.example` to `.env`, and run
   `docker compose -f apps/keel/compose.yaml up -d`.
2. Sign in as the owner, register a passkey, add your OpenPGP key as a trusted signer, and copy
   `keel ssh-ca show` and your passkey's public key into a scratch copy of the Ansible vars.
3. Bind a repository on git.boop.ninja (Forgejo receiver) with `.keel/jobs.yaml`, signed mode on.
4. Apply the base profile to a throwaway VM with `-e keel_enroll_token=…`. The host appears with
   its profiles.
5. Approve its baseline with the passkey. Push a signed commit: it runs without review. Push an
   unsigned one: review shows the diff, the passkey approval runs it, and the host page shows the
   new applied revision.
6. Stop `keel-runner` on the VM mid-run. The host becomes `lost` after about 45 s.
7. Revoke the host, then destroy the VM. Run `docker compose -f apps/keel/compose.yaml down`.
   Volumes are left in place; remove them only by name, by hand.
