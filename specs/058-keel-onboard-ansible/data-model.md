# Data model: onboarding the Ansible repo to Keel (058)

Most of 058 is host state and repository files, not database rows. The one new stored entity
belongs to Keel (amendment A2).

## Host-side state (per onboarded host)

| Item | Where | Owner / mode | Lifecycle |
|---|---|---|---|
| Human automation group | `automation` group, `/etc/sudoers.d/automation` | root `0440` | Unchanged by Keel. Narrowed by base hardening ([base-hardening](contracts/base-hardening.md)). |
| Approved composes | `/etc/automation/composes.list` | root `0644` | Rewritten by `base` from `base_automation_composes`. |
| Executor account | user and group `keel-exec`, nologin, home `/var/lib/keel-executor` `0700` | – | Created by `keel_runner` (R3). Never created on a host that has not run `make keel-node`. |
| Root grant | `/etc/sudoers.d/keel-exec` | root `0440` | Written only after the prerequisite assertions pass. Removed on uninstall. |
| Trial leftovers | `automation` user with home `/var/lib/keel-executor`, `/etc/sudoers.d/keel-automation` | – | Migrated once (R4), then absent. |
| SOPS age key | `/etc/sops/age/keys.txt`, symlink at the old path | `root:keel-exec` `0440`, dir `0750` | Moved once (R7). |
| Outside-run log | `/var/lib/keel-outside/runs.jsonl` | root `0644` | Append-only, from the callback. Read by the runner ([keel-amendments](contracts/keel-amendments.md) A2). |
| Runner read offset | runner state dir | runner | Advanced only on `OutsideRunsAck`. |

**Executor state transitions**

```text
none ──make keel-node──▶ installed (no root grant)
installed ──prereqs pass──▶ granted
installed ──prereqs fail──▶ installed (role fails, grant absent)
trial(automation) ──make keel-node──▶ migrated ──prereqs──▶ granted
granted ──base regresses (rule or ACL reappears)──▶ role refuses on next run, grant removed
```

## Repository entities (`../ansible`)

- **Profile**: name ∈ {debian, prox-vm, gameserver, host, k3s-master, k3s-agent, keel-server,
  keel-node}.
- **Job**: playbook, profiles, inventory `fleet`, `paths` (or none = manual), timeout. See
  [jobs-manifest](contracts/jobs-manifest.md).
- **Trust set**: signers, approvers, quorum, require_signed (fleet-wide), plus allowed profiles
  per functional group. See [trust-vars](contracts/trust-vars.md).
- **Inventory host**: its name equals its hostname; it belongs to `keel_nodes` and to exactly one
  functional group.

## Keel (fathom) additions

**OutsideRun** (table `host_outside_runs`, insert-only)

| Field | Type | Rule |
|---|---|---|
| id | uuid | PK |
| host_id | uuid | FK hosts |
| at | timestamptz | host's claim |
| user | text | ≤ 64 bytes |
| playbook | text | ≤ 256 bytes |
| commit | text, nullable | 40 hex when present |
| dirty | bool | |
| check | bool | |
| received_at | timestamptz | server time |

Unique on `(host_id, at, playbook, coalesce(commit,''))`, so a duplicate entry is ignored.

**Host.changed_outside** (bool, derived and stored)

```text
clear ──entry with check=false──▶ set
set ──baseline approved, or every assigned job completed after newest entry──▶ clear
```
