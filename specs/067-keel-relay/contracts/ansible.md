# Contract: the operator's Ansible repository (`../ansible`)

As in spec 055 (`contracts/host-install.md`), this is what Keel relies on. The roles are written
in `../ansible`, not here, and only with the owner's go-ahead (separate tasks, separate commits in
that repository).

## `keel_runner` role: new variables

| Variable | Default | Notes |
|---|---|---|
| `keel_runner_dial` | `[]` | Ordered `host:port` list the runner dials (`--dial`). Empty: dial `keel_runner_server` as today. |
| `keel_runner_download_origin` | `keel_runner_conn.origin` | Where `/runner/<release>/…` is fetched from. A relay's `http://<relay>:7880` is allowed: every file is still checked against `keel_runner_sha256`. |
| `keel_runner_queries` | unset | `{ kinds: [...], units: [...], paths: [...] }`. Unset: `queries.toml` is absent and every query is refused. |

Validation added to the role:

- `keel_runner_dial` non-empty requires `keel_runner_server` (FR-008). Each entry must match
  `host:port`.
- `keel_runner_queries.kinds` entries must be among the five kinds; `paths` must be absolute.
- `queries.toml` is written only when the installed runner version supports `query-v1`, so an
  older runner never sees it (it would ignore it anyway: separate file, research R8).

The unit's `ExecStart` gains one `--dial` per entry.

## `keel_relay` role (new) and `keel-relay` profile

| Variable | Default | Notes |
|---|---|---|
| `keel_relay_enabled` | `false` | |
| `keel_relay_version`, `keel_relay_sha256` | required | Same release as the runner; checksums per architecture. |
| `keel_relay_hub_keys` | required, non-empty | Keel's relay-link public lines, from Settings → Relays. Written to `/etc/keel-relay/hub_keys`. |
| `keel_relay_link_listen` | `0.0.0.0:7832` | Address Keel dials. |
| `keel_relay_runner_listen` | `0.0.0.0:7831` | Address the zone's runners dial. |
| `keel_relay_http_listen` | unset | Enables the release cache. |
| `keel_relay_max_streams` | `256` | |

What the role does:

1. Creates system user `keel-relay` (no shell, no home login).
2. Installs `/usr/local/bin/keel-relay`, checked against `keel_relay_sha256` (fail closed on a
   placeholder or mismatch, like `binaries.yml`).
3. Writes `/etc/keel-relay/relay.toml` and `hub_keys` (root-owned, `0644`).
4. Installs and enables `keel-relay.service` (hardening as in research R9), as a host service,
   never as a cluster workload.
5. Runs `keel-relay key` and prints the public line and fingerprint with `debug`, for the owner
   to paste into Settings → Relays.
6. Opens `link_listen` to Keel's egress address only, and `runner_listen` / `http_listen` to the
   zone's hosts, when the host firewall role is in use.

The `keel-relay` profile is `base` + `keel_relay`. A host may carry both `keel-runner` and
`keel-relay`.
