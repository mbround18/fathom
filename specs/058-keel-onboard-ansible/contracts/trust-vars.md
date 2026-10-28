# Contract: trust variables in `../ansible` (US4, FR-014, FR-015; research R11)

The variable names and shapes are those `roles/keel_runner/defaults/main.yml` already defines.
This contract fixes where each one is set, and what value it takes.

## Fleet-wide: `inventory/group_vars/keel_nodes.yml`

| Variable | Value | Notes |
|---|---|---|
| `keel_runner_require_signed` | `true` | Required on every onboarded host (FR-015). |
| `keel_runner_trusted_signers` | ≥ 1 entry | SSH `allowed_signers` lines (principal `keel`, namespace `git`) or armored OpenPGP keys. Public keys only. |
| `keel_runner_approvers` | ≥ 1 entry | Each exactly `{label, credential_id, public_key_cose, alg}`, copied from Keel's passkey page. |
| `keel_runner_override_quorum` | `1` | Raised only when a second human approver exists. |
| `keel_runner_max_override_age` | `24h` | Default, restated. |
| `keel_runner_server`, `_origin`, `_host_ca` | empty | A join token fills them. An enroll-token run supplies them at run time. |
| `keel_runner_release`, `keel_runner_sha256` | pinned version, real checksums | Bumping them is the upgrade (FR-008). |

Forbidden here: `keel_runner_allowed_profiles` (see below), and any IP address (FR-009).

## Per functional group: `keel_runner_allowed_profiles`

| Group file | Value |
|---|---|
| `compose_hosts.yml` | `[prox-vm, keel-node]` |
| `pve_hosts.yml` | `[host, keel-node]` |
| `k3s_servers.yml` | `[k3s-master, keel-node]` |
| `k3s_agents.yml` | `[k3s-agent, keel-node]` |
| `keel_servers.yml` | `[keel-server, keel-node]` |
| gameserver hosts (`host_vars` or a `gameservers` group) | `[gameserver, keel-node]` |

- Each node is in exactly one of these groups. Ansible merges group vars at the same depth
  alphabetically, so a value in `keel_nodes.yml` would silently win over some groups and lose to
  others.
- `debian` is in no group's list. It is for hosts that have only the baseline. A host with a
  bigger profile already applies `base` through that profile.

## Trial hosts only: ignored `inventory/host_vars/<name>.yml`

```yaml
keel_runner_require_signed: false
keel_runner_allowed_profiles: [debian, keel-node]
```

## Checks (in `make lint`, a script under `scripts/`)

- `keel_nodes.yml` has `require_signed: true`, at least one signer and approver, no
  `allowed_profiles`, and no `REPLACE_WITH` placeholder.
- Every host listed under `keel_nodes` resolves `allowed_profiles` from exactly one group file.
- No IPv4 or IPv6 literal appears in `inventory/` (the existing check, extended).
- The executor role's own parse, as in 055 `signed-mode.md`, is the final gate: any malformed
  approver makes it refuse the whole file.
