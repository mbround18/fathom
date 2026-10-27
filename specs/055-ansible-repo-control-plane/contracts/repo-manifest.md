# Contract: repository manifest (`.keel/jobs.yaml`)

The repository declares which jobs exist. Keel decides whether they run and who approves them
(research R6). The manifest is versioned with `version: 1`, and unknown keys are an error, so a
typo cannot quietly drop a setting.

```yaml
version: 1
inventories:
  prod:
    path: inventories/prod/hosts.yaml     # YAML or INI; group_vars/ and host_vars/ next to it
    schema: inventories/schema.json       # optional JSON Schema for host and group vars
jobs:
  web-deploy:
    playbook: playbooks/web.yaml
    inventory: prod                       # the name above, or a managed inventory's name in the workspace
    limit: "web:&prod"                    # Ansible host pattern; resolved and shown before approval
    vars: { release_channel: stable }     # extra vars; must not contain credential values
    credentials: [prod-ssh, prod-vault]   # credential names; each must be granted to this binding
    paths: ["playbooks/web.yaml", "roles/web/**", "inventories/prod/**"]   # changes that trigger a run; omit for manual-only
    timeout: 30m
    rollback:
      playbook: playbooks/web-rollback.yaml   # and/or:
      reapply: last_success
```

## Validation

The manifest is validated at sync time. A manifest that fails validation leaves the previous
job set active, and the binding shows `manifest_invalid` with the errors.

- Every path stays inside the repository: no `..`, no absolute paths, and no symbolic link that
  leaves the tree.
- `playbook` exists at the commit.
- `limit` parses as an Ansible pattern, and resolves to at least one host in the pinned revision.
- `credentials` names exist and are granted. A name that is not granted makes the job
  `blocked (credential_unavailable)`; it is not silently dropped.
- `vars` keys may not be named like `ansible_*password*`, `ansible_ssh_private_key*` or
  `ansible_become_pass*`. Those values come only from credentials.

Nothing in the manifest can set or relax approval, step-up or runner-pool policy.
