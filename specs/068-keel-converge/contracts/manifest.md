# Contract: `.keel/jobs.yaml` additions (spec 068)

Parsed by `keel_core::manifest` on the server (sync) and on the runner (every job). Every key
below is optional (FR-016). `deny_unknown_fields` stays everywhere. `version` stays `1` (research
R4, owner question Q4).

**Compatibility**: a runner older than 068 refuses the whole manifest as soon as any key below
appears. Upgrade runners first (quickstart §4).

## `profiles.<p>.includes` and `profiles.<p>.jobs`

```yaml
profiles:
  debian:
    description: "Baseline"
    jobs: [debian]
  keel-node:
    description: "Keel's runner"
    includes: [debian]
    jobs: [keel-node]
  k3s-master:
    includes: [debian]          # optional; k3s-master.yaml already runs base, so usually omitted
    jobs: [k3s-master]
```

- `includes`: profile names, applied first, in order, recursively.
- `jobs`: job names, applied next, in order.
- A profile without `includes`/`jobs` has no recipe and keeps today's meaning: a converge of a host
  with only that profile runs every job whose `profiles` contains it, in manifest (alphabetical)
  order. This keeps old manifests convergeable.
- **Expansion** (`converge::expand`): depth-first over the host's assigned profiles in name
  order; each job appears once, at its first position.
- **Refused** at validation: unknown profile or job name; a cycle (`profile cycle: a → b → a`); a
  job listed twice in one recipe; expansion > 32 steps; depth > 8.

## `groups:`

```yaml
groups:
  compose_hosts: [prox-vm]
  gameservers: [gameserver]
  k3s_servers: [k3s-master]
  k3s_agents: [k3s-agent]
  pve_hosts: [host]
  keel_servers: [keel-server]
  keel_nodes: [keel-node]
```

- Keys: inventory group names (not validated against the inventory, which can change
  independently; an unknown group simply matches no host). Values: profile names (must exist).
- Used only to compute suggestions (FR-009). Never assigns.

## `jobs.<j>.covers`

```yaml
jobs:
  k3s-master:
    covers: [k3s-audit, k3s-addons, k3s-io-priority, k3s-backup]
  k3s-agent:
    covers: [k3s-addons]
  gameserver:
    covers: [prox-vm]
```

- Names must be jobs in the manifest. Closure is transitive. Refused: self-cover, cycles.
- Meaning: running this job at commit C applies everything the covered job would at C. The repo
  author asserts this; `make lint` in the operator repo is the place to check it (contracts/ansible.md).

## `jobs.<j>.env`

```yaml
jobs:
  tailscale:
    env:
      TS_AUTHKEY: tailscale-authkey     # credential name in the binding's workspace
  k3s-agent:
    env:
      K3S_URL: k3s-url
      K3S_TOKEN: k3s-token
```

- Name: `^[A-Z_][A-Z0-9_]{0,63}$`, not on the refusal list (`ANSIBLE_*`, `LD_*`, `PYTHON*`,
  `KEEL_*`, `PATH`, `HOME`, `SHELL`, `IFS`, `ENV`, `BASH_ENV`), shared in `keel_core::env_names`.
  ≤ 16 entries.
- Value: a credential **name**. Sync warns (not refuses) when the credential does not exist; the
  run fails at dispatch with `credential_unavailable`.
- A credential referenced by `env:` must be of the new kind `env_value` (one value, no name;
  contracts/protocol.md). The existing `env` kind (NAME=value lines, bound per binding) is
  refused here, so one credential never means two things.

## Param kind `choice`

```yaml
jobs:
  provision:
    params:
      stack: { kind: choice, from: composes }          # directory names under composes/ at the commit
  something:
    params:
      size: { kind: choice, values: [small, medium, large] }
```

- Exactly one of `values` (1–64 entries) or `from` (a repo-relative directory; no `..`, no
  absolute path; must exist at the commit).
- Values match `^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$`; with `from`, entries that don't match are
  ignored, hidden directories (`.x`) are ignored, and > 256 entries is refused.
- Optional `default:` must be one of the values (with `from`, checked at sync for the commit).
- The executor re-checks the value against its own checkout (`choice_mismatch`).

## Manual-only jobs

Unchanged: a job without `paths` never runs on a push. `provision` is one.

A `choice` param with no `default` is **required** (T074), and a job with one is
**manual-only**: a push, a converge and a baseline send no params, so only Start can run it.

- A required param with `paths:` is refused (`required_param_paths`).
- A recipe (`profiles.<p>.jobs`) listing a manual-only job is refused (`manual_only_recipe_job`).
- A profile without a recipe still reaches it, and a converge skips it: the preview lists it
  under the host's `skipped` (`needs_value`), not as a step and not as a refusal.
- Start without the value is refused `param_invalid` ("The run's param stack is required.").
- `oci_digest` params stay optional.

## Validation errors (new codes)

`profile_cycle`, `unknown_include`, `unknown_recipe_job`, `duplicate_recipe_job`,
`recipe_too_long`, `unknown_group_profile`, `unknown_cover`, `cover_cycle`, `self_cover`,
`env_name_invalid`, `env_name_reserved`, `env_too_many`, `choice_shape`, `choice_from_missing`,
`choice_from_too_many`, `choice_default`, `required_param_paths`, `manual_only_recipe_job`.
