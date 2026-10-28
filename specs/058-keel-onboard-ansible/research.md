# Research: Onboard the personal Ansible repository onto Keel

Decisions for [plan.md](plan.md). "The repository" is `../ansible` (git.boop.ninja); "the trial
branch" is its `keel-runner-trial` branch, the starting point (spec Assumptions). 055 references
are to `specs/055-ansible-repo-control-plane/`.

## R1. The `/root/ansible` checkout: read-only to the group

**Decision**: `/root/ansible` stays root-owned, and the `automation` group gets read and traverse
only (ACL `r-X`, recursive and default), including `.git`. The exact `git pull --ff-only`,
`git status --short` and `git fetch --all` sudo rules stay until US6 cleanup, still run as root,
and now run against metadata only root can write.

**Rationale**: There are three paths to root through this directory, not two. Write access to `.git`
lets a member plant a hook or a command-running config key (`core.hooksPath`, `core.fsmonitor`,
`core.sshCommand`, filter and diff drivers) that root then runs on `git pull`. Write access to the
working tree lets a member edit a playbook that root then runs on the next break-glass `make`.
Removing write on `.git` alone closes the first and leaves the second. The base user is
already in `sudo`, so nobody loses a workflow they need: the group existed so that `git pull`
could happen without a password, and the sudo rules still allow exactly that. Under Keel, the
executor never touches `/root/ansible` (it has its own Git store), so the checkout becomes
break-glass only.

**Alternatives considered**: Running root's `git` with `-c core.hooksPath=/dev/null
-c core.fsmonitor=false` was rejected: the list of command-running keys is long, and it grows.
Excluding only `.git` from the ACL was rejected because it leaves the working-tree path open.
Dropping the group entirely was rejected for now: the compose rule (R2) and the pull rules
still need it until US6.

## R2. The compose rule: a fixed-list wrapper

**Decision**: Replace `docker compose -f /root/ansible/composes/* up -d` with
`/usr/local/sbin/automation-compose-up NAME`. It is root-owned `0755`, and sudo allows it with any
arguments because the wrapper does the checking:

- exactly one argument, matching `^[a-z0-9][a-z0-9-]{0,62}$`;
- the name must be a line in `/etc/automation/composes.list` (root-owned `0644`), which the base
  role writes from `base_automation_composes` (default: the stack this host is named after, if
  `composes/<hostname>/compose.yaml` exists);
- it then runs `docker compose -f /root/ansible/composes/NAME/compose.yaml up -d` with a fixed
  argv, a cleared environment (`env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin`) and no shell.

**Rationale**: A `*` in a sudoers rule matches spaces and slashes, so the current rule accepts
`… /root/ansible/composes/x up -d -f /tmp/evil.yaml up -d`. A wrapper with one validated
argument removes caller-supplied paths and flags entirely. Because R1 makes the compose files
root-writable only, what the wrapper starts is what the repository holds.

**Alternatives considered**: sudoers regex arguments (sudo 1.9.10+) were rejected because they
are easy to get wrong in the same way, and they don't help on older hosts. Allowing the
`docker` group was rejected: that is root-equivalent outright.

## R3. Executor account `keel-exec`, and how the root grant is gated

**Decision**: The executor runs as `keel-exec`: a system user whose primary group is also
`keel-exec`, a member of `keel-trust`, with shell `/usr/sbin/nologin`, home
`/var/lib/keel-executor` (`0700`) and no human in any of its groups. Its sudo rule is
`/etc/sudoers.d/keel-exec`, with `keel-exec ALL=(root) NOPASSWD: ALL` and
`Defaults:keel-exec !requiretty`. The rule is written only after the role asserts the
prerequisites on the live host:

1. `/etc/sudoers.d/automation` contains no `docker compose` rule, and does contain the wrapper
   rule (R2);
2. `getfacl /root/ansible/.git` and `getfacl /root/ansible` show no `w` for the `automation`
   group, in either the access or the default ACL (R1), or `/root/ansible` does not exist.

If either check fails, the role fails and names which one. It does not fall back to installing
the runner without root, because a runner that cannot `become` fails every real job anyway.

**Rationale**: The role checks the state of the host, not a marker file, so a host where someone
re-added the old rule by hand is caught. 055's contract already intends the executor to have its
own primary group (host-install.md, "Accounts"); the name collision is why that never happened.

**Alternatives considered**: Renaming the human group was rejected by the owner. A marker file
written by the base role was rejected because it can lie.

## R4. Migrating hosts that ran the trial role

**Decision**: The runner role detects the trial account: a user named `automation` whose home is
`/var/lib/keel-executor` and whose shell is `nologin`. When it finds one, it:

1. stops `keel-runner.service` and the executor socket, waiting for any running
   `keel-executor@` instance to finish (bounded by the job deadline);
2. creates `keel-exec`;
3. runs `chown -R keel-exec:keel-exec /var/lib/keel-executor`, keeping the ledger and the Git
   store;
4. removes the `automation` user only, never the `automation` group, and removes
   `/etc/sudoers.d/keel-automation` if the trial wrote one;
5. installs the new units and starts them.

A human account named `automation` (any other home or shell) is never touched; the role fails
instead.

**Rationale**: FR-019 requires keeping the ledger, which holds the host's applied-revision
history. Matching on home and shell together keeps the role from deleting a real person's
account.

## R5. One command: `make keel-node` runs `debian` first

**Decision**: The `keel-node` Makefile target applies `playbooks/debian.yaml` and then
`playbooks/keel-node.yaml`, in one invocation, through `KEEL_PLAYBOOK`, so both runs use the
hostname as the inventory name. The Keel job `keel-node` runs only `playbooks/keel-node.yaml`;
the `debian` job owns the base. The documented line becomes `KEEL_TOKEN=<token> make keel-node`,
and `make debian keel-node` keeps working.

**Rationale**: It matches the owner's flow with one command, and it keeps the two Keel jobs from
doing each other's work on every run.

**Alternatives considered**: Importing `debian.yaml` inside `keel-node.yaml` was rejected: every
runner upgrade would then rerun the whole base on every node.

## R6. Inventory names under `make` and under Keel

**Decision**: The inventory name is the hostname, everywhere. `KEEL_PLAYBOOK` already passes
`-i "$(hostname),"`, and `keel_runner_inventory_name` defaults to `inventory_hostname`. The role
fails if the host is not listed in `inventory/hosts.yml` under that name and Keel is in use
(FR-013), because the executor's forced `--limit` would otherwise match nothing and every job
would silently do nothing. The off-LAN k3s nodes keep their addresses in ignored `host_vars`
(FR-009).

## R7. SOPS secrets under Keel

**Decision**: The age key moves from `/root/.config/sops/age/keys.txt` to
`/etc/sops/age/keys.txt`:

- directory `0750` root:keel-exec, file `0440` root:keel-exec when the runner is installed;
- root-only `0400` otherwise;
- the old path stays as a symlink.

`community.sops.load_vars` is given `age_keyfile: "{{ sops_age_key_file }}"` explicitly.

**Rationale**: `load_vars` runs on the controller. Under Keel the controller is `keel-exec`,
before any `become`, so it cannot read a key under `/root` (mode `0700`). Giving `keel-exec`
read access grants nothing new: it is already root through sudo (R3). The key never leaves the
host, as today.

**Alternatives considered**: Decrypting in a `become` task was rejected: `load_vars` is an action
plugin and does not support it. Keeping secrets on the Keel server was rejected: that is
spec 059's job, and the constitution forbids Keel holding material that reaches hosts.

## R8. Detecting a run outside Keel (055 amendment)

**Decision**: The repository's `ansible.cfg` enables a repo-local callback,
`callback_plugins/keel_outside.py`. On every playbook run where `KEEL_EVENT_FD` is not set (so
not started by the executor), on a host where `/var/lib/keel-outside/` exists, it appends one
JSON line to `/var/lib/keel-outside/runs.jsonl` with:

- the time, the user and the playbook;
- `git rev-parse HEAD` of the checkout and whether the tree is dirty;
- whether the run was `--check`.

The directory is root `0755` and the file root `0644`, created by the runner role. The callback
never fails a run: any error is swallowed after one warning line.

On connect, and every 60 s, the runner reads the file from its last acknowledged offset and sends
the new entries in a new frame, `OutsideRuns { entries }`. Keel then:

- stores them per host;
- shows the host as **changed outside Keel**, with the entries;
- keeps that flag until the owner either re-baselines the host or runs its jobs again.

`--check` runs are recorded but do not set the flag.

**Rationale**: 055 tracks drift as "commits behind the branch", which a break-glass run does not
change. FR-017 and SC-005 need a signal Keel can see within 5 minutes of returning. Recording
this on the host, in the repository's own Ansible configuration, works without Keel, Keycloak
or the Git server.

**Limits, stated in the docs**: The record is advisory. Root can delete it, and an
`ansible-playbook` run that bypasses the repository's `ansible.cfg` is not recorded.

**055 impact**: a runner frame, a host field and event, a UI badge and filter, round-trip tests
and generated bindings (Principle II/III). Planned as a 055 amendment, not built inside 058.

## R9. Keel jobs that restart what Keel depends on

**Decision**:

- **Keel server host**: the `keel_server` role writes the compose project and pulls images. When
  `KEEL_EVENT_FD` is set, it defers `docker compose up -d` the same way `keel_runner` defers its
  restart (a systemd transient timer that starts 30 s after the job's result is reported).
  Otherwise it restarts at once.
- **`forgejo` and `keycloak` hosts**: no deferral. Their jobs run like any other. Recovery is
  break-glass (`make prox-vm` or `make provision` on the box), and the order in US5 means
  break-glass has been proven before these hosts are enrolled.

**Rationale**: A run that kills its own server mid-run cannot report its result. Deferral makes
the result land first. For Forgejo and Keycloak, a failed run is visible and recoverable without
Keel, which is the point of break-glass.

## R10. Profiles, jobs and trigger paths

**Decision**: As in [contracts/jobs-manifest.md](contracts/jobs-manifest.md):

- one profile and one commit-triggered job per target in FR-010;
- `upgrade` and `clean` as manual-only jobs (no `paths`);
- each job's paths are its playbook, the roles it uses (directly or through `import_playbook`
  and `include_role`), the `group_vars` of the groups it targets, and the files every run reads
  (`ansible.cfg`, `requirements.yml`, `inventory/group_vars/all.yml`).

`inventory/hosts.yml` triggers nothing, because adding a host must not run every job everywhere.
A host's first run is its baseline anyway.

**Rationale**: Paths that are too broad cause noisy runs, and paths that are too narrow miss
changes. Deriving paths from the role graph is mechanical and testable: a lint check in the
repository compares each job's paths with the roles its playbook resolves to.

## R11. Trust per host group

**Decision**: Trust lives in `inventory/group_vars/keel_nodes.yml`, the group every node is
added to, with narrower `allowed_profiles` set in each functional group's vars
(`compose_hosts`, `pve_hosts`, `k3s_servers`, `k3s_agents`, `keel_servers`), as in
[contracts/trust-vars.md](contracts/trust-vars.md).

- Signers, approvers, quorum and `require_signed: true` are fleet-wide.
- `override_quorum` starts at 1 and stays 1 until a second human approver exists. Two passkeys
  held by the owner on different devices count as recovery, not as a second approver.
- Trial hosts set `keel_runner_require_signed: false` in their own `host_vars`, which are ignored
  by Git, so that setting can never be committed for a real host.
- `keel_nodes.yml` MUST NOT set `keel_runner_allowed_profiles`. Group vars at the same depth
  merge alphabetically, so `keel_nodes` would override `compose_hosts`, `k3s_*` and
  `keel_servers` but lose to `pve_hosts`. The trial's `[debian]` moves to the trial host's
  ignored host_vars. A lint check asserts that each `keel_nodes` member gets
  `allowed_profiles` from exactly one functional group.

## R12. Onboarding order and each host's gate

**Decision**:

1. Trial VM (unsigned, disposable).
2. One ordinary compose host (`audiobookshelf` or `pulse`), signed. Prove break-glass on it
   (US6 test).
3. The remaining ordinary compose hosts (`rustfs`, `steam`).
4. The Keel server host.
5. `forgejo`.
6. `keycloak`.
7. `pve`.
8. The k3s agent.
9. The k3s server.

Each host must pass four gates before the next one starts:

- it is enrolled;
- its baseline is approved at the commit it was last applied from;
- its first run of every job shows no unexplained changes;
- a `make` run on it shows up as **changed outside Keel**.

US6 cleanup runs after step 9.

**Rationale**: Low-risk hosts first, hosts Keel depends on only after break-glass has been
proven, and the hypervisor and Kubernetes last (spec Assumptions).

## R13. Constitution wording

**Decision**: A PATCH amendment, 1.4.0 → 1.4.1. In "Keel host runner", change "jobs run as the
automation account" to "jobs run as a dedicated executor account that no human shares". No
principle changes.
