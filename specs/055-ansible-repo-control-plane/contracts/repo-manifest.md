# Contract: repository manifest (`.keel/jobs.yaml`)

The repository declares which jobs exist. Keel decides whether they run and who approves them
(research R6). The manifest is versioned with `version: 1`, and unknown keys are an error, so a
typo cannot quietly drop a setting.

```yaml
version: 1
profiles:                                 # kinds of host (research R21); hosts are assigned these in Keel
  debian: { description: "Every Debian host: the base profile" }
  gameserver: { description: "Game servers" }
inventories:
  prod:
    path: inventories/prod/hosts.yaml     # YAML or INI; group_vars/ and host_vars/ next to it
    schema: inventories/schema.json       # optional JSON Schema for host and group vars
jobs:
  web-deploy:
    playbook: playbooks/web.yaml
    profiles: [debian]                    # hosts with any of these profiles run this job (required)
    inventory: prod                       # the name above, or (not in signed mode) a managed inventory
    limit: "web:&prod"                    # optional Ansible pattern, intersected with the profile's hosts
    vars: { release_channel: stable }     # extra vars; must not contain credential values
    credentials: [prod-vault, web-secrets]   # credential names; each must be granted to this binding
    paths: ["playbooks/web.yaml", "roles/web/**", "inventories/prod/**"]   # changes that trigger a run; omit for manual-only
    timeout: 30m
    rollback:
      playbook: playbooks/web-rollback.yaml   # and/or:
      reapply: last_success
```

## On the host

Each host runs a job against itself only: `--limit <its inventory name>` and
`ansible_connection=local`, both forced by the executor. A job's `limit` only narrows which hosts
get the job. In signed mode the host reads this file from the signed tree and ignores Keel's copy
(research R19), so changing a job's playbook, profiles or vars is a commit like any other.

## Validation

The manifest is validated at sync time. A manifest that fails validation leaves the previous
job set active, and the binding shows `manifest_invalid` with the errors.

- Every path stays inside the repository: no `..`, no absolute paths, and no symbolic link that
  leaves the tree.
- `playbook` exists at the commit.
- `profiles` is non-empty and names profiles declared above.
- `limit`, if present, parses as an Ansible pattern.
- In signed mode, `inventory` names a repository inventory.
- `credentials` names exist and are granted. A name that is not granted makes the job
  `blocked (credential_unavailable)`; it is not silently dropped.
- `vars` keys may not be named like `ansible_*password*`, `ansible_ssh_private_key*` or
  `ansible_become_pass*`. Those values come only from credentials.

Nothing in the manifest can set or relax approval, step-up, signed mode, quorum or a host's
allowed profiles. Declaring a profile does not assign it to any host.
