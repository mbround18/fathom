# Contract: `.keel/jobs.yaml` in `../ansible` (US3, FR-010–FR-012; research R10)

The format is 055's `contracts/repo-manifest.md`, version 1. This contract fixes the content.

## Shared trigger paths

Every commit-triggered job also lists `ansible.cfg`, `requirements.yml` and
`inventory/group_vars/all.yml`. `inventory/hosts.yml` triggers nothing: adding a host must not
run every job.

## Profiles and jobs

| Job / profile | Playbook | Roles (incl. `include_role`) | Group vars | Trigger |
|---|---|---|---|---|
| `debian` | `playbooks/debian.yaml` | base | – | paths |
| `prox-vm` | `playbooks/prox-vm.yaml` | base, cleanup, storage, docker, tailscale, devtools, github_release | `compose_hosts.yml` | paths |
| `gameserver` | `playbooks/gameserver.yaml` (imports `prox-vm.yaml`) | storage + the `prox-vm` set | `compose_hosts.yml` | paths |
| `host` | `playbooks/pve.yaml` | base | `pve_hosts.yml` | paths |
| `k3s-master` | `playbooks/k3s-master.yaml` | base, k3s_addons, k3s_node, k3s_audit, sops, k3s_backup | `k3s_servers.yml`, `secrets/k3s_servers.sops.yaml` | paths |
| `k3s-agent` | `playbooks/k3s-agent.yaml` | base, k3s_addons, k3s_node | `k3s_agents.yml` | paths |
| `keel-server` | `playbooks/keel-server.yaml` | base, docker, keel_server | `keel_servers.yml` | paths |
| `keel-node` | `playbooks/keel-node.yaml` | keel_runner | `keel_nodes.yml` | paths |
| `upgrade` | `playbooks/upgrade.yaml` | topgrade, github_release | – | manual only (no `paths`) |
| `clean` | `playbooks/cleanup.yaml` | cleanup | – | manual only (no `paths`) |

- Each role appears in `paths` as `roles/<name>/**`; each playbook appears as itself. A playbook
  that reads `playbooks/files/…` lists those files too.
- `upgrade` and `clean` run on every profile. Their `profiles` list is the eight profiles above.
- Not jobs (FR-012): `provision`, `template`, `tailscale`, `secrets`, `secrets-check`,
  `secrets-edit`, `lint`, `test`, `k3s-addons`, `k3s-audit`, `k3s-backup`.
- Timeouts: `keel-node` 30m, `upgrade` 60m, `clean` 30m, `k3s-*` 45m, others 30m.

## Conditions on the playbooks themselves

- Every job's playbook MUST run unattended: no `vars_prompt`, no `pause`, and no required
  run-time variable. In particular:
  - the `tailscale` role inside `prox-vm` MUST be a no-op on a host that is already on the
    tailnet when `TS_AUTHKEY` is unset;
  - `keel-node` under Keel never has `KEEL_TOKEN`.
- `keel-server` defers its `compose up` when `KEEL_EVENT_FD` is set (research R9).

## Check (`scripts/keel-manifest-check.py`, run by `make lint`)

For each job with `paths`:

- Parse its playbook, its `import_playbook`s, its `roles:`, and every
  `include_role`/`import_role` with a literal name, recursively.
- Fail if a role or group-vars file that the job reaches is missing from `paths`.
- Fail if a path matches no file.
- Fail if a job uses a profile that the manifest does not declare.

Done in pure Python 3 against the repo, with no Ansible import, so it runs in the
existing lint step.
