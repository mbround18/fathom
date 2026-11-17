# Contract: the operator's Ansible repository (`../ansible`)

As in spec 055 (`contracts/host-install.md`), this is what Keel relies on. The roles are written
in `../ansible`, not here. The fluxified Claude session writes them (owner's request, 2026-11-17),
with separate commits in that repository. The text below is what was agreed with that session where
the role differs from this contract's first draft.

## `keel_runner` role: new variables

| Variable | Default | Notes |
|---|---|---|
| `keel_runner_dial` | `[]` | Ordered `host:port` list the runner dials, written to `runner.env` as `KEEL_RUNNER_DIAL=<a>,<b>`. Empty: dial `keel_runner_server` as today. |
| `keel_runner_download_origin` | `keel_runner_conn.origin` | Where `/runner/<release>/…` is fetched from. A relay's `http://<relay>:7880` is allowed: every file is still checked against `keel_runner_sha256`. |
| `keel_runner_queries` | unset | `{ kinds: [...], units: [...], paths: [...] }`. Unset: `queries.toml` is absent and every query is refused. |

Validation added to the role:

- `keel_runner_dial` non-empty requires `keel_runner_server` (FR-008). Each entry must match
  `host:port`.
- `keel_runner_queries.kinds` entries must be among the five kinds; `paths` must be absolute.
- `queries.toml` is written (`root:keel-trust 0640`, like the other trust files) when
  `keel_runner_queries` is set, and removed when it is unset. There is no version gate: an older
  runner ignores the file (separate file, research R8).

The unit file is unchanged: it stays a byte-for-byte copy of
`apps/keel/runner/systemd/keel-runner.service`, whose `ExecStart` takes no flags. The dial list
arrives through `runner.env`, so `keel-runner`'s `--dial` must read `KEEL_RUNNER_DIAL`, split on
`,`.

## `keel_relay` role (new) and `keel-relay` profile

| Variable | Default | Notes |
|---|---|---|
| `keel_relay_enabled` | `false` | |
| `keel_relay_version`, `keel_relay_sha256` | required | Same release as the runner. `keel_relay_sha256` keys are `<arch>/keel-relay`, as in the release `SHA256SUMS`. |
| `keel_relay_origin` | `keel_runner_conn.origin` | Fetched on the controller (the relay host cannot reach Keel): `<origin>/runner/<version>/<arch>/keel-relay`. |
| `keel_relay_download`, `keel_relay_binary_dir` | `true`, unset | `false` installs from a local build, as `keel_runner` does. |
| `keel_relay_hub_keys` | required, non-empty | Keel's relay-link public lines, from Settings → Relays. Written to `/etc/keel-relay/hub_keys`. |
| `keel_relay_link_listen` | `0.0.0.0:7832` | Address Keel dials. |
| `keel_relay_runner_listen` | `0.0.0.0:7831` | Address the zone's runners dial. |
| `keel_relay_http_listen` | unset | Enables the release cache. |
| `keel_relay_max_streams` | `256` | |

What the role does:

1. Creates system user `keel-relay` (no shell, no home login).
2. Downloads the binary on the controller (`delegate_to: localhost`), checks it against
   `keel_relay_sha256` (fail closed on a placeholder or mismatch, like `binaries.yml`), and copies
   it to `/usr/local/bin/keel-relay`.
3. Writes `/etc/keel-relay/relay.toml` and `hub_keys` (root-owned, `0644`).
4. Installs and enables `keel-relay.service` (hardening as in research R9), as a host service,
   never as a cluster workload.
5. Runs `sudo -u keel-relay keel-relay key` and prints its output with `debug`, for the owner to
   paste into Settings → Relays. `keel-relay key` creates `/var/lib/keel-relay/host_ed25519`
   (`0600`) if it is missing, and prints exactly two lines: the OpenSSH public line, then
   `SHA256:<fingerprint>`.
6. No firewall step: `../ansible` has no host firewall role, and Cilium owns the packet path on
   the k3s nodes. The role documents the exposure instead: restrict `link_listen` (7832) to the
   home egress address at OVH's edge firewall, and `runner_listen` / `http_listen` to the zone's
   hosts.

The `keel-relay` profile follows the repository's pattern: a `.keel/jobs.yaml` profile
`keel-relay`, `playbooks/keel-relay.yaml`, and `make keel-relay`. As with `keel-node`, the make
target applies `debian.yaml` (base) first, and the job runs only `keel-relay.yaml`. A host may
carry both `keel-runner` and `keel-relay`.
