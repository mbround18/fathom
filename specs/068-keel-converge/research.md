# Research: Keel host provisioning and convergence (spec 068)

Read on 2026-11-17 against this branch (from 067-keel-relay after D1/D2) and `../ansible` at
`aadf5d5` ("keel_relay: role, playbook and keel-relay profile"). File:line references are to
those trees. Each finding ends with the decision it led to.

## R1. What `../ansible` actually does (the patterns to support)

### Make targets → recipes

| Target | Makefile | What it runs | Keel today | 068 |
|---|---|---|---|---|
| `debian` | :76 | `playbooks/debian.yaml` (role `base`) | job `debian` | unchanged |
| `prox-vm` | :80 | `prox-vm.yaml`: base, cleanup, storage, docker, tailscale, devtools in one play | job `prox-vm` | unchanged; recipe `[prox-vm]` |
| `gameserver` | :73 | `gameserver.yaml` imports `prox-vm.yaml` | job `gameserver` | recipe `includes: [prox-vm]`, `jobs: [gameserver]` (needs R1a) |
| `host` | :70 | `pve.yaml` | job `host` | unchanged |
| `tailscale` | :94 | `tailscale.yaml`; key from `TS_AUTHKEY` or minted from `../fluxified` | none | narrow job, `env: {TS_AUTHKEY: …}` |
| `provision` | :105 | `provision.yaml`; `ansible.builtin.pause` (:71) when `stack_name` is undefined | none | `stack` choice param replaces the pause |
| `template` | :112 | shell script on the hypervisor | none | out of scope (spec Assumptions) |
| `k3s-master` | :158 | `k3s-master.yaml`: base, k3s_addons, k3s_io_priority, k3s_node, k3s_audit, sops, k3s_backup | job `k3s-master` | unchanged, `covers` the four narrow jobs |
| `k3s-agent` | :162 | `k3s-agent.yaml`: base, k3s_addons, k3s_node; new agents need `K3S_URL`, `K3S_TOKEN` | job `k3s-agent` | `covers: [k3s-addons]`, `env` for the two |
| `k3s-audit` | :171 | `k3s-audit.yaml` (hosts `k3s_servers`) | none | narrow job |
| `k3s-addons` | :177 | `k3s-addons.yaml` (hosts `k3s_servers:k3s_agents`) | none | narrow job |
| `k3s-io-priority` | :183 | `k3s-io-priority.yaml` | none | narrow job |
| `k3s-backup` | :189 | `k3s-backup.yaml`: sops, k3s_backup | none | narrow job |
| `keel-server` | :215 | `keel-server.yaml` (base, docker, sops, keel_server); `KEEL_DIGEST` | job + `oci_digest` param | unchanged |
| `keel-node` | :218 | **`debian.yaml` then `keel-node.yaml`** in one invocation | job `keel-node` runs only `keel-node.yaml` | recipe `includes: [debian]`, `jobs: [keel-node]` |
| `keel-relay` | :221 | **`debian.yaml` then `keel-relay.yaml`** | job `keel-relay` | recipe `includes: [debian]`, `jobs: [keel-relay]` |
| `upgrade` | :250 | `upgrade.yaml` | manual job | unchanged |
| `clean` | :255 | `cleanup.yaml` | manual job | unchanged |

Only `keel-node` and `keel-relay` chain playbooks in make. The others bundle roles inside one
playbook, so they need `covers`, not recipes, to stop a narrow job running beside them.

**R1a, correction to the spec's framing.** `gameserver.yaml` already imports `prox-vm.yaml`, so a
`gameserver` recipe `includes: [prox-vm]` would run prox-vm twice (once as its own step, once
inside gameserver). The recipe is just `jobs: [gameserver]`, with `covers: [prox-vm]` on the
`gameserver` job. Only add `includes` where make chains *separate* playbooks.

### Ceilings (`keel_runner_allowed_profiles`)

Set per functional group, never in `keel_nodes.yml` (`inventory/group_vars/keel_nodes.yml:8`):

| Group | File | Ceiling |
|---|---|---|
| compose_hosts | `compose_hosts.yml:10` | `[prox-vm, keel-node]` |
| gameservers | `gameservers.yml:8` | `[gameserver, keel-node]` |
| k3s_agents | `k3s_agents.yml:10` | `[k3s-agent, keel-node]` |
| pve_hosts | `pve_hosts.yml:15` | `[host, keel-node]` |
| keel_servers | `keel_servers.yml:39` | `[keel-server, keel-node]` |
| k3s_servers | `k3s_servers.yml:31` | `[k3s-master, keel-node]` |

**Finding: no ceiling allows `debian` or `keel-relay`.** A recipe `keel-node → includes debian`
expands to a step whose job `debian` declares `profiles: [debian]`. The runner refuses it on every
host (`runner/src/executor/mod.rs` `policy.allows_any(&def.profiles)`). See R5.

### Environment lookups

| Variable | Where | 068 |
|---|---|---|
| `TS_AUTHKEY` | `roles/tailscale/defaults/main.yml:48` | `env:` binding on `tailscale`, `prox-vm`, `gameserver` |
| `K3S_URL`, `K3S_TOKEN` | `roles/k3s_node/defaults/main.yml:76-77` | `env:` binding on `k3s-agent` |
| `KEEL_TOKEN` | `roles/keel_runner/vars/main.yml:58` | enrollment only; stays refused (`KEEL_*`) |
| `KEEL_EVENT_FD` | `keel_runner/vars/main.yml:52`, `keel_server/vars/main.yml:6` | set by the executor; unchanged |
| `HOME`, `MOLECULE_*` | `keel_relay` vars and molecule | not under Keel |

`TS_AUTHKEY`, `K3S_URL` and `K3S_TOKEN` are not on the executor's refusal list
(`runner/src/executor/secrets.rs:71-76`: `ANSIBLE_*`, `LD_*`, `PYTHON*`, `KEEL_*`, `PATH`, `HOME`,
`SHELL`, `IFS`, `ENV`, `BASH_ENV`). No change to the list.

### The prompt

`playbooks/provision.yaml:70-79` pauses for a stack name when the hostname is not one of
`composes/*` (`compose_stack_names`, :55). Under Keel stdin is `/dev/null`, so `pause` returns an
empty string at once and the play fails at :84 with "isn't one of composes/*". That is the right
outcome (FR-015 is already mostly met), but the operator gets no way to pick. Decision: the
ansible delta adds `stack` as an extra var that skips the pause, and Keel declares a `choice`
param `stack: {kind: choice, from: composes}`. Stacks today: audiobookshelf, forgejo, keycloak,
pulse, rustfs, steam.

## R2. The per-host lock (FR-003)

`queue.rs:1601` `dispatch_ready` picks the oldest queued `run_hosts` row per host (`distinct on
(q.host_id) … order by r.created_at, r.id`), skips hosts with a row in `sent`/`running`, and locks
with `for update of rh skip locked`. The backstop is 0016's
`run_hosts_one_in_flight on run_hosts (host_id) where state in ('sent','running')`.

That lock is per **step**. Between step N finishing and step N+1 being claimed, the host has no
in-flight row, so an older queued path run would be picked first (it sorts earlier by
`created_at`). The chain needs a lock that spans steps.

Options considered:

1. **Advisory locks** (`pg_advisory_lock(host)`): session-scoped, lost on connection churn in the
   pool; not visible to queries. Rejected.
2. **A `holding` row per converge host, with a partial unique index** — chosen:
   - `converge_hosts (run_id, host_id, state)` where state is
     `waiting | holding | succeeded | failed | cancelled`.
   - `create unique index converge_hosts_one_holding on converge_hosts (host_id) where state = 'holding'`.
   - `dispatch_ready`'s candidate subquery gains
     `and not exists (select 1 from converge_hosts ch where ch.host_id = q.host_id and ch.state = 'holding' and ch.run_id is distinct from r.parent_run_id)`.
   - Claiming a converge's first step for a host flips its `converge_hosts` row `waiting →
     holding` in the same transaction. The unique index makes two holders impossible; a losing
     claim rolls back and the host stays queued.
   - The hold is released (`holding → succeeded|failed|cancelled`) when the host's chain ends.
3. **Pre-create all steps as `queued` and rely on ordering**: breaks as soon as a path run is
   created between two steps. Rejected.

**Advancing the chain.** Step outcomes are written from five places: `finished()` (:1133),
`disconnected()` (:1299), `recover_in_flight()` (:1365), `fail_host` and `approvals::cancel_hosts`
(:218). Patching each is fragile. Decision: an `AFTER UPDATE OF state ON run_hosts` trigger,
`converge_advance()`, which fires only when a row of a converge child run moves from a
non-terminal state (`blocked|queued|sent|running`) to a terminal one. It either unblocks that
host's next step (`blocked → queued`) or, on anything but `succeeded`, marks the rest `skipped`
(reason `dependency_failed`) and releases the hold. Because it fires only on non-terminal →
terminal, a late `Finished` replayed onto a `lost` row cannot advance twice. `recover_in_flight`
gains a reconcile pass as the backstop (holds with no non-terminal step are closed).

A trigger is new in this codebase. Alternative: one Rust function `converge::advance(tx, run_host)`
called from the five sites, with a test per site. Recorded as open question Q5; the plan assumes
the trigger because it cannot be forgotten by a sixth writer.

## R3. Signed mode and one approval (FR-005)

- `runs::create_run` (`runs.rs:298`) runs `review::early_check` in signed mode, which yields the
  review groups for the run.
- The host-side challenge is `signed::run_context(workspace, binding, run_id, job, quorum,
  params)`: an override or baseline assertion is bound to **one run id and one job**.

So "approve once" is achievable for the **policy approval** (spec 064: one `runs` row with
`approvals_needed`), but override/baseline passkey assertions remain per step unless keel-trust
learns to sign a plan digest. Decision for 068: child runs and their ids exist at plan time, the
review page lists every step's assertion together, and the approver can sign them in one sitting
(one WebAuthn prompt per assertion). A plan-digest challenge is open question Q2.

## R4. Old runners and the strict manifest (FR-016)

The executor parses `.keel/jobs.yaml` itself (`runner/src/executor/mod.rs`, `manifest::validate`)
with `keel_core::manifest`, which is `deny_unknown_fields` everywhere. A runner built before 068
**refuses every job** of a manifest that uses any new key (`includes`, `groups`, `covers`, `env`,
`choice`), including jobs that don't use them. FR-016's "strict parsing still applies" therefore
implies a rollout order:

1. Ship server and runner 068 (they parse the new keys).
2. Upgrade every runner (the `keel-node` job does this).
3. Only then does the ansible delta land.

Mitigations in 068:

- Sync computes, per binding, the hosts whose `runner_version` is below the first 068 runner
  version and shows "N runners would refuse this manifest" on the binding (warning, not refusal).
- Dispatch to a runner whose `Hello` lacks capability `manifest-v068` fails the host fast with
  refusal `runner_too_old` rather than letting it parse and refuse. 067 adds `Hello.capabilities`.
- New Assign fields (`check`, `env` items) are skipped when unused, and an old runner
  (`deny_unknown_fields` on `Frame`) fails closed when they are present.

Bumping the manifest to `version: 2` would make the failure explicit on old runners but forces
every repo to change. Open question Q4; the plan keeps `version: 1`.

## R5. Effective profiles (FR-017 vs recipes)

A recipe step is still a job, and the runner checks the **job's** profiles against its ceiling.
For `keel-node → includes [debian]`, the `debian` job (profiles `[debian]`) is refused on every
host (R1 ceilings).

Options:

- **A. Derive.** keel-core `validate` computes each job's *effective profiles*: its declared
  profiles plus every profile whose recipe reaches it. The runner checks effective profiles. A
  host with ceiling `[prox-vm, keel-node]` then accepts `debian` *as reached through keel-node*.
  This widens what a ceiling means by what the repo writes, which FR-017 forbids in spirit
  ("converge never widens what a host accepts"): the repo, not the host, would decide that
  `keel-node` implies `debian`.
- **B. Ceiling change.** The ansible delta adds `debian` (and `keel-relay` where relays run) to
  every group's ceiling. The host decides; nothing in Keel widens. Costs one line per group file.

Recommendation: **B**, because the host stays the authority. Open question Q1 for the owner.
The plan and contracts are written for B; A is a contained change in `manifest::validate` and
the executor's check if the owner picks it.

## R6. Coverage (FR-010)

`covers` is transitive at validation (a covers b, b covers c ⇒ a covers c), refuses cycles and
unknown names, and a job never covers itself. At `create_run` for a path-triggered run of job J:
for each target host with a `converge_hosts` row in `waiting|holding` whose remaining steps
(`converge_steps` not terminal) include J or a job that covers J, and whose commit equals the
run's commit or descends from it (`Mirror` ancestry; the mirror is already used for `pin`), the
host's row is inserted as `covered` with `covered_by = <converge run id>` and audited. Other
hosts queue normally.

The reverse case (a converge planned after a queued path run that it covers) is **not** folded:
the path run is older and runs first, then the converge. Simple and correct; noted in the UI.

## R7. Check mode (FR-011)

- `runs.check_mode boolean not null default false`.
- `Frame::Assign` gains `check: bool` with `skip_serializing_if = "is_false"`; old runners fail
  closed (R4).
- The executor appends `--check --diff`.
- **Ledger.** `keel_trust::ledger::Outcome` (`crates/keel-trust/src/ledger.rs:48`) has
  `Succeeded`/`Failed` (move the base) and `Cancelled` (does not). A check run must keep replay
  protection (its `start` line consumes the challenge) but must not move the base. Decision: new
  variant `Checked` that does not move the base. This is a keel-trust change and gets its own
  unit tests (base unchanged after `Checked`; replay of the same run id refused).
- The server skips `host_revisions`, drift and rollback-point writes in `finished()` when
  `runs.check_mode`.
- Policy: `PolicyMatch` (`crates/keel-core/src/policy.rs:28`) gains `check: Option<bool>`; `None`
  matches both, so existing policies apply unchanged to check runs.

## R8. Environment credentials (FR-012)

- Credentials kinds (`crates/keel-protocol/src/binding.rs:173`) already include `Env`, whose value
  is `NAME=value` lines. That is a whole-env blob chosen per binding, not per job.
- 068's `env: { NAME: credential }` names a credential per variable. Values resolve through the
  `SecretResolver` port (`crates/keel-core/src/ports.rs:97`).
- **Correction:** today `queue.rs:1840` `secrets()` opens credentials directly through
  `crate::credentials::open`, not through the port. 068 routes the new `env:` lookups (and, as a
  contained refactor, the existing ones) through `SecretResolver`, so spec 059's adapter applies.
- Wire: `SecretItem` gains kind `env_var { name }` carrying one value; the executor puts it in
  `ansible-playbook`'s environment after `env_clear`, re-checking the name against the refusal
  list. Old runners fail closed on the unknown kind.
- Canary test (SC-005): a credential `CANARY-068-…` used via `env:`; grep the run log, events,
  audit and every text/jsonb column for it.

## R9. Commit-status gate (FR-013)

**Correction to the spec's assumption** ("The forge adapter from spec 066 can read commit
statuses"). The `Forge` port (`crates/keel-core/src/ports.rs:218`) has only `probe` (:220) and
`open_change` (:223). Neither `crates/keel-forge` adapter reads statuses. 068 adds
`fn commit_statuses(&self, repo, sha) -> ForgeFuture<Vec<CommitStatus>>`, implemented for
Forgejo (`GET /repos/{o}/{r}/commits/{sha}/statuses`, combined) and GitHub (combined status +
check runs). A forge without the method returns `Unsupported`, and the binding preview refuses to
add required checks.

Runs from a commit enter `waiting_checks`; a poller (the existing sync loop's cadence) moves them
to `queued`, or ends them `failed` with `state_reason = checks_failed` or `checks_timeout`.

## R10. `choice` params (FR-014)

`ParamKind` (`manifest.rs:145`) has only `OciDigest`; `MAX_PARAMS = 8` (:133). `RepoTree` exposes
only `kind(path)`, no listing. 068 adds `RepoTree::list_dirs(path)` (the server implements it
from the mirror/`repo_catalog`; the runner from its checkout). `choice` takes either `values:
[...]` or `from: <repo dir>`; values match `^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$`. The server
checks at preview against the pinned commit; the executor re-checks against its own checkout.
`PreviewRunStart { job, limit }` has no params today, so the Start dialog gains params.

## R11. Enrollment and profile edits (FR-006, FR-008, FR-009)

- `runner_listener.rs:447` `enrol()` inserts `hosts` and `host_profiles` from the enrollment's
  profiles, marks it used, calls `hosts::issue_cert` (`hosts.rs:369`, principals `host:<name>` and
  `profile:<p>` for assigned ∩ allowed) and audits `host_enrolled`. The converge is created
  **after** that transaction commits (a failed plan must not undo enrollment), as actor
  `system:enroll`, only when `KEEL_CONVERGE` is on and `host_enrollments.auto_converge`.
- Join-request acceptance (FR-006 second half) depends on 067 D3 (`T058`, not built). The 068 task
  hooks the same function into acceptance and is blocked until D3 lands.
- Profile edits: new `PreviewHostProfiles` → `ConfirmHostProfiles` (step-up, admin), writing
  `host_profiles` with `assigned_by`, re-issuing the cert, auditing in one transaction.
- Suggestions: sync expands `groups:` against the pinned inventory's `inventory_hosts.groups` and
  upserts `profile_suggestions`. The host page shows them; accepting one opens the same preview.

## R12. Migration number

`apps/keel/server/migrations` has 0001–0014 and 0016. 067 reserves `0015_join.sql` (D3) and
`0017_relays.sql`. 068 takes **`0018_converge.sql`**. Risk: if 067 renumbers or another branch
lands an 0018 first, rename on merge; sqlx checks the checksum of applied migrations, so a rename
is only safe before any shared database has applied it.

## R13. FR-015 (no controlling terminal)

`runner/src/executor/process.rs:185-194` already does `env_clear`, `stdin(Stdio::null())` and
`setpgid(0,0)`. It does not call `setsid`, so a play that opens `/dev/tty` (e.g. `pause` with
`echo: false`, or a become password prompt) can still reach the runner's terminal when the runner
runs in a foreground shell. Decision: `setsid()` instead of `setpgid(0,0)` (a new session is also a
new process group, so group kill still works), with a test that runs a fixture playbook doing a
`pause` and asserts it fails within 5 s.
