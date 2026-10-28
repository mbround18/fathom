# Implementation Plan: Onboard the Ansible repo to Keel

**Branch**: `main` (058 work lands on `../ansible` integration branches) | **Date**: 2026-10-27 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/058-keel-onboard-ansible/spec.md`

## Summary

Hand the home fleet's Ansible repo (`../ansible`) to Keel host by host, without opening a path to
root that Keel's checks don't cover. There are three strands:

1. **Harden `roles/base`.** The `automation` group gets read-only access to `/root/ansible`, and
   the compose wildcard is replaced by a fixed-list wrapper (R1, R2).
2. **Amend Keel (055).**
   - A1: the executor runs as a dedicated `keel-exec` account, and gets root only after the base
     hardening is verified on the host (R3, R4).
   - A2: the runner reports runs made outside Keel, so Keel can flag the host (R8).
3. **Shape the repo for Keel.**
   - one command, `KEEL_TOKEN=… make keel-node` (R5, R6);
   - SOPS readable by the executor (R7);
   - eight profiles and jobs whose trigger paths are checked (R10);
   - trust set in group vars (R11);
   - a gated onboarding order (R12).

## Technical Context

**Language/Version**:
- Ansible (ansible-core, the version pinned by the repo's uv lock) and POSIX sh, in `../ansible`.
- Rust (stable, edition 2024) for the Keel runner, server and protocol.
- TypeScript/React for the Keel UI.
- Python 3 for the callback plugin and the manifest check.

**Primary Dependencies**: Existing ones only: Molecule (Docker driver) for role tests, `sqlx`,
the `keel-protocol` crate, `community.sops`. Nothing new.

**Storage**:
- Keel's PostgreSQL gains a migration: `host_outside_runs` and `hosts.changed_outside`.
- On hosts, flat files (data-model.md).

**Testing**:
- Molecule for `base` and `keel_runner`;
- `cargo test -p keel-protocol -p keel-runner -p keel-server`;
- Vitest for UI units, and one Playwright spec for the badge;
- `make lint`, extended with the manifest and trust checks.

**Target Platform**: Debian 12/13 hosts (VMs, the Proxmox host, k3s nodes). The Keel server runs
as a Linux container.

**Project Type**: Infrastructure repository plus amendments to an existing web service and
agent.

**Performance Goals**: An outside run shows in Keel within 60 s of reconnecting. The manifest
check finishes in under 5 s on the repo.

**Constraints**:
- No IPs in git, and no secrets logged; `no_log` on tokens.
- Fail closed: the root grant is absent unless every prerequisite holds.
- Break-glass works without Keel, Keycloak or Forgejo.
- One host at a time.

**Scale/Scope**: About a dozen hosts and eight profiles: one repo change set, and two Keel
amendments.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Assessment |
|---|---|
| I. Operator-first, explicit mutation | Pass. Every host is onboarded by hand and baselined. Jobs only run what the operator already runs with `make`. Outside runs are surfaced, never hidden or blocked. |
| II. One contract per app | Pass. `OutsideRuns`/`OutsideRunsAck` go into the existing runner channel (`keel-exec` frames). The host fields go into `keel-protocol`, with regenerated bindings. No side channel. |
| III. Tested at the boundaries | Pass. Molecule covers the privilege boundary (sudoers, ACL, wrapper, root grant). Round-trip tests cover the new frames. A server test covers the flag. Playwright covers the badge. |
| IV. Density scales with width | Pass. The only UI is one badge and a filter in existing host views, which follow existing layouts. |
| V. Fast and quiet | Pass. A batch every 60 s, of at most 100 entries; no polling from the UI (it's event-driven). |
| VI. User data is sacred | Pass. `host_outside_runs` is insert-only and audited. The migration of trial hosts keeps the ledger byte-identical, never deletes the `automation` group, and fails closed. |
| Platforms: Keel host runner tier | **Needs the PATCH 1.4.0 → 1.4.1** (R13). Its wording "jobs run as the automation account" becomes "a dedicated executor account that no human shares". This is a clarification, not a change of rule, and it must land before A1. |

Post-design re-check: still passes. The design adds no new dependency or service, and no
mutation without an operator. Nothing goes in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/058-keel-onboard-ansible/
├── plan.md
├── research.md          # R1–R13
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── base-hardening.md
│   ├── keel-amendments.md   # A1 keel-exec, A2 outside runs
│   ├── jobs-manifest.md
│   └── trust-vars.md
└── tasks.md             # /speckit-tasks
```

### Source code

```text
# fathom (this repo): amendments A1, A2
.specify/memory/constitution.md                 # 1.4.1
specs/055-ansible-repo-control-plane/           # contract edits (runner, host-install, …)
crates/keel-exec/src/frames.rs                  # OutsideRuns, OutsideRunsAck (runner channel)
crates/keel-protocol/src/host.rs                # changed_outside, entries, ListHosts filter, event
apps/keel/runner/src/                           # outside-run reader, offset, keel-exec naming
apps/keel/runner/systemd/                       # User=keel-exec, RestartPreventExitStatus=78
apps/keel/server/migrations/                    # host_outside_runs, hosts.changed_outside
apps/keel/server/src/                           # ingest, flag set/clear, audit, GetHost/ListHosts
apps/keel/web/src/                              # Changed outside Keel badge and filter
apps/keel/web/tests/                            # one Playwright spec

# ../ansible (integration branch)
roles/base/                                     # ACL r-X, sudoers aliases, wrapper, composes.list
roles/keel_runner/                              # keel-exec, prereq gate, trial migration, SOPS key, outside dir
roles/keel_server/                              # deferred compose up under Keel
roles/tailscale/                                # no-op without TS_AUTHKEY when already up
callback_plugins/keel_outside.py                # outside-run record
playbooks/*.yaml                                # load_vars age_keyfile, keel-node inventory name check
inventory/group_vars/{keel_nodes,compose_hosts,pve_hosts,k3s_*,keel_servers}.yml
.keel/jobs.yaml                                 # eight profiles, ten jobs
scripts/keel-manifest-check.py, scripts/keel-trust-check.py
Makefile                                        # keel-node runs debian then keel-node; lint runs the checks
```

**Structure decision**:
- The Keel amendments follow 055's existing layout.
- The repo changes stay inside the roles and playbooks they affect, plus two lint scripts.
- They land on a `../ansible` integration branch, and the trial branch's commits are
  cherry-picked onto it.
- Order of landing: the constitution PATCH and A1, then base hardening, then the rest of the
  repo, then A2, then onboarding by R12's order.

## Complexity Tracking

No violations.
