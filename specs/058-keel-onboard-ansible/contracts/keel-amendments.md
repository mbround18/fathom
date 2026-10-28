# Contract: amendments to 055 (Keel) that 058 needs

Two changes to Keel itself, made in this repository (fathom) before onboarding any real host.
Each is a 055 contract edit plus code. They are listed here so that 058's tasks own them.

## A1. Executor account `keel-exec` (FR-002, FR-004, FR-019; research R3, R4)

**055 documents**: In `contracts/runner.md` and `contracts/host-install.md` ("Accounts", the units,
"Uninstall"), plus `plan.md`, `quickstart.md`, `research.md` and `spec.md`, the executor account
`automation` becomes `keel-exec`. The sudoers file becomes `/etc/sudoers.d/keel-exec`. Two
things are added to host-install.md: the prerequisite check (research R3), and the trial
migration (research R4).

**Code**:

- `apps/keel/runner/systemd/keel-executor@.service`: `User=keel-exec`, `Group=keel-exec`.
- `apps/keel/runner/src/bin/keel-executor.rs`, `src/lib.rs` and `src/executor/mod.rs`: any literal
  account name or doc comment.
- `apps/keel/runner/systemd/keel-runner.service`: add `RestartPreventExitStatus=78` (055
  join.md).
- Re-vendor both units into `../ansible/roles/keel_runner/files/systemd/`.

**Constitution**: PATCH 1.4.0 → 1.4.1 (research R13).

**Tests**:

- The executor's existing tests keep passing.
- A unit-file test asserts `User=keel-exec` and `RestartPreventExitStatus=78`.
- Molecule, in `roles/keel_runner`:
  - fresh install;
  - migration from a trial `automation` user, with the ledger bytes unchanged afterwards;
  - refusal when a human `automation` user exists;
  - refusal when each prerequisite is missing.

## A2. Runs outside Keel (FR-017, SC-005; research R8)

### Host side (the runner role and the runner)

- The role creates `/var/lib/keel-outside/` (root `0755`) and `runs.jsonl` (root `0644`).
- Each line is a record:

  ```json
  {"v":1,"at":"2026-10-27T12:00:00Z","user":"root","playbook":"playbooks/debian.yaml",
   "commit":"<40 hex or null>","dirty":true,"check":false}
  ```

  Every field is bounded: `user` ≤ 64 bytes, `playbook` ≤ 256 bytes, the whole line ≤ 1 KiB.
  The runner skips a line that is too long or malformed, and counts it as skipped.
- The runner keeps its read offset in its state directory. On `Hello`, and every 60 s while
  connected, it sends what is new. If the file shrinks (truncated or replaced), it reads from the
  start, and Keel de-duplicates by `(at, playbook, commit)`.

### Runner channel (`crates/keel-exec/src/frames.rs`, subsystem `keel-runner-v1`)

| Dir | Frame | Rules |
|---|---|---|
| R → S | `OutsideRuns { entries: Vec<OutsideRun>, skipped: u32 }` | At most 100 entries per frame. |
| S → R | `OutsideRunsAck { through: u64 }` | Byte offset; the runner advances only on ack. |

### Server

- Table `host_outside_runs` (insert-only): host id, the fields of the record, and when it was
  received. Keel audits each batch it receives.
- A host gains `changed_outside: bool`:
  - set by any entry with `check: false`;
  - cleared by a baseline approval for the host, or by a completed run of every job assigned to
    the host after the newest entry.

### Web protocol and UI

- `GetHost` adds `changed_outside` and the newest 20 entries. `ListHosts { changed_outside? }`
  filters on it. Event: `HostChangedOutside { host }`.
- The UI shows a **Changed outside Keel** badge on the host row and in the host view, with the
  entries labelled as the host's claims.

### Tests

- Round-trip tests for the new frames and fields; generated bindings regenerated.
- A runner test covers the offset, truncation and malformed lines.
- A server test covers setting, clearing and de-duplicating the flag.
- A Playwright spec covers the badge appearing live, the filter, and clearing by baseline.
