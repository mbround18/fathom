# Contract: what `../ansible` changes for spec 068

For the ansible session. Keel's side is in [manifest.md](manifest.md). Read at `../ansible`
`aadf5d5`. **Nothing here lands before every runner is on 068** (research R4): a pre-068 runner
refuses the whole manifest once it has any of these keys. The `keel-node` job is the upgrade path;
once Keel's binding page shows "0 runners would refuse this manifest", merge.

Also blocked on owner question Q1 (ceilings vs derived profiles). This contract is written for
the recommended answer, ceilings (§5).

## 1. Recipes for the nine profiles (mirrors the Makefile)

```yaml
profiles:
  debian:      {description: "...", jobs: [debian]}                          # make debian (:76)
  prox-vm:     {description: "...", jobs: [prox-vm]}                         # make prox-vm (:80)
  gameserver:  {description: "...", jobs: [gameserver]}                      # make gameserver (:73); imports prox-vm.yaml, so no include
  host:        {description: "...", jobs: [host]}                            # make host (:70)
  k3s-master:  {description: "...", jobs: [k3s-master]}                      # make k3s-master (:158)
  k3s-agent:   {description: "...", jobs: [k3s-agent]}                      # make k3s-agent (:162)
  keel-server: {description: "...", jobs: [keel-server]}                     # make keel-server (:215)
  keel-node:   {description: "...", includes: [debian], jobs: [keel-node]}  # make keel-node (:218) runs debian.yaml then keel-node.yaml
  keel-relay:  {description: "...", includes: [debian], jobs: [keel-relay]}  # make keel-relay (:221) runs debian.yaml then keel-relay.yaml
```

Single-playbook profiles get a one-job recipe so a converge is explicit, not "every job naming
this profile" (which for `debian` would also pull in `upgrade` and `clean`).

## 2. Narrow jobs

Each runs its own playbook. Paths follow the file's own rule (playbook, roles reached, group vars
of the hosts it targets, plus the three every run reads).

```yaml
  k3s-audit:
    playbook: playbooks/k3s-audit.yaml           # make k3s-audit (:171); hosts k3s_servers
    profiles: [k3s-master]
    inventory: fleet
    paths: [ansible.cfg, requirements.yml, inventory/group_vars/all.yml,
            inventory/group_vars/k3s_servers.yml, playbooks/k3s-audit.yaml, roles/k3s_audit/**]
    timeout: 20m

  k3s-addons:
    playbook: playbooks/k3s-addons.yaml          # make k3s-addons (:177); hosts k3s_servers:k3s_agents
    profiles: [k3s-master, k3s-agent]
    inventory: fleet
    paths: [ansible.cfg, requirements.yml, inventory/group_vars/all.yml,
            inventory/group_vars/k3s_servers.yml, inventory/group_vars/k3s_agents.yml,
            playbooks/k3s-addons.yaml, roles/k3s_addons/**]
    timeout: 20m

  k3s-io-priority:
    playbook: playbooks/k3s-io-priority.yaml     # make k3s-io-priority (:183)
    profiles: [k3s-master]
    inventory: fleet
    paths: [ansible.cfg, requirements.yml, inventory/group_vars/all.yml,
            inventory/group_vars/k3s_servers.yml, playbooks/k3s-io-priority.yaml,
            roles/k3s_io_priority/**]
    timeout: 10m

  k3s-backup:
    playbook: playbooks/k3s-backup.yaml          # make k3s-backup (:189); sops + k3s_backup
    profiles: [k3s-master]
    inventory: fleet
    paths: [ansible.cfg, requirements.yml, inventory/group_vars/all.yml,
            inventory/group_vars/k3s_servers.yml, secrets/k3s_servers.sops.yaml,
            playbooks/k3s-backup.yaml, roles/sops/**, roles/k3s_backup/**]
    timeout: 20m

  tailscale:
    playbook: playbooks/tailscale.yaml           # make tailscale (:94)
    profiles: [prox-vm, gameserver]
    inventory: fleet
    env: {TS_AUTHKEY: tailscale-authkey}
    paths: [ansible.cfg, requirements.yml, inventory/group_vars/all.yml,
            inventory/group_vars/compose_hosts.yml, playbooks/tailscale.yaml, roles/tailscale/**]
    timeout: 15m
```

**Path overlap, intended.** `roles/k3s_audit/**` is in both `k3s-master` and `k3s-audit`. A
change to it triggers both; Keel folds `k3s-audit` into `k3s-master` per host only if a
`k3s-master` run or converge is queued/running there (`covers`). Otherwise both run, one after
the other under the per-host lock. To get SC-003 ("only its narrow job"), **remove the narrow
roles from the bundled job's `paths`**: `k3s-master` then triggers on `roles/k3s_node/**`,
`roles/base/**` and its playbook, and a change confined to `roles/k3s_audit/**` triggers only
`k3s-audit`. Recommended; the ansible session decides, and `scripts/keel-manifest-check.py`
needs a rule for it ("a role reached by a bundled job may be left out of its paths when a narrow
job it covers lists it").

## 3. `covers`

```yaml
  k3s-master: {covers: [k3s-audit, k3s-addons, k3s-io-priority, k3s-backup]}
  k3s-agent:  {covers: [k3s-addons]}
  prox-vm:    {covers: [tailscale]}
  gameserver: {covers: [prox-vm, tailscale]}
```

`make lint` should check that a covering job's playbook reaches every role of each job it covers
(the manifest check already walks roles).

## 4. `groups:`

```yaml
groups:
  compose_hosts: [prox-vm]
  gameservers:   [gameserver]
  pve_hosts:     [host]
  k3s_servers:   [k3s-master]
  k3s_agents:    [k3s-agent]
  keel_servers:  [keel-server]
  keel_nodes:    [keel-node]
```

`debian` is not mapped: every host gets it through `keel-node`'s recipe.

## 5. Ceilings (Q1, recommended option B)

Add `debian` to every functional group's `keel_runner_allowed_profiles`:

| File | Line | Now | After |
|---|---|---|---|
| `inventory/group_vars/compose_hosts.yml` | 10 | `[prox-vm, keel-node]` | `[prox-vm, keel-node, debian]` |
| `inventory/group_vars/gameservers.yml` | 8 | `[gameserver, keel-node]` | `[gameserver, keel-node, debian]` |
| `inventory/group_vars/k3s_agents.yml` | 10 | `[k3s-agent, keel-node]` | `[k3s-agent, keel-node, debian]` |
| `inventory/group_vars/pve_hosts.yml` | 15 | `[host, keel-node]` | `[host, keel-node, debian]` |
| `inventory/group_vars/keel_servers.yml` | 39 | `[keel-server, keel-node]` | `[keel-server, keel-node, debian]` |
| `inventory/group_vars/k3s_servers.yml` | 31 | `[k3s-master, keel-node]` | `[k3s-master, keel-node, debian]` |

Relay hosts additionally need `keel-relay` in their ceiling (host vars, as the role is
configured per host). The ceilings take effect when the `keel-node` job runs, so this change
lands **before** §1 (otherwise the first converge of a `keel-node` host refuses its `debian`
step).

## 6. Env bindings

| Var | Role | Job(s) | Keel credential (kind `env_value`) |
|---|---|---|---|
| `TS_AUTHKEY` | `roles/tailscale/defaults/main.yml:48` | `tailscale`, `prox-vm`, `gameserver` | `tailscale-authkey` |
| `K3S_URL` | `roles/k3s_node/defaults/main.yml:76` | `k3s-agent` | `k3s-url` |
| `K3S_TOKEN` | `roles/k3s_node/defaults/main.yml:77` | `k3s-agent` | `k3s-token` |

`KEEL_TOKEN` stays out: it only enrolls a runner, before Keel can run anything (spec
Assumptions). The Headscale key minting (`../fluxified … mint_node_key`) stays a make-only path;
under Keel the operator stores a reusable or pre-auth key as `tailscale-authkey`. Whether Keel
should mint per run is out of scope.

## 7. `provision.yaml`: `stack` instead of the pause

`playbooks/provision.yaml:70-79` pauses for a stack name. Change it so that an extra var `stack`,
when set, is used directly (and validated against `compose_stack_names` as at :84), and the
`pause` only runs when neither `stack` nor a matching hostname is available **and** stdin is a
TTY. Then:

```yaml
  provision:
    playbook: playbooks/provision.yaml
    profiles: [prox-vm, gameserver]
    inventory: fleet
    params:
      stack: {kind: choice, from: composes}
    timeout: 30m
    # no paths: manual only
```

## 8. Not changed

`upgrade`, `clean` (manual), `keel-server`'s `oci_digest` param, `template` (outside Keel), the
`KEEL_EVENT_FD` checks in the roles.
