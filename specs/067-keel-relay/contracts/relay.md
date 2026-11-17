# Contract: the relay link (`keel-relay-v1`) and runner additions

What `keel-relay`, the Keel server and `keel-runner` agree on. The runner SSH contract (spec 055
`contracts/runner.md`) is unchanged except for the additive items under "Runner".

## Topology

```text
 runner ──TCP──► relay:7831 (runner_listen)          relay:7832 (link_listen) ◄──SSH── Keel
                   │                                          ▲
                   └── forwarded-tcpip channel "keel-runner" ─┘  (runner's SSH session inside)
```

Keel → relay is one SSH connection. Every runner session is one channel on it. Nothing is ever
dialled from the relay to Keel, or from Keel to a runner.

## Link SSH connection

- **Algorithms**: keel-exec's lists (`server_preferred` on the relay, `client_preferred` on Keel).
  Ed25519 only, `curve25519-sha256`, AEAD ciphers, strict KEX, no compression.
- **Relay host key**: a plain Ed25519 key (no certificate). Keel accepts exactly the key pasted
  for that relay (`relays.host_key`); any other key ends the handshake, is audited as
  `relay.refused { reason: "host_key_mismatch", fingerprint }`, and the relay is **quarantined**
  (see "Keel never trusts the relay" below).
- **Keel auth**: user `keel`, public-key auth with the active relay-link key. The relay accepts
  only keys in its `hub_keys` file; any other user name or key is refused after the usual
  rejection delay. Password, keyboard-interactive and host-based auth are not offered.
- **Keepalive**: SSH keepalive every 15 s from both ends; 3 misses close the link.

## Channels

| Opened by | Type | Accepted when | Otherwise |
|---|---|---|---|
| Keel | `session` + subsystem `keel-relay-v1` | Exactly one per link, first channel. | Refused; a second one closes the link. |
| Relay | `forwarded-tcpip`, connected address `keel-runner`, port `0` | After the control channel is up; under `max_streams`. | Refused by Keel and audited. |
| either | anything else (`direct-tcpip`, `x11`, `auth-agent`, `session` from the relay), any `exec`/`shell`/`pty`/`env`/`tcpip-forward` request, any global request | never | Refused; the relay refuses and logs; Keel refuses and audits `relay.refused`. |

On a `forwarded-tcpip` channel the originator address/port is the runner's TCP peer **as the
relay reports it**. Keel records it as advisory and never uses it for a decision.

Keel runs the runner listener's own SSH server (`server_config`, `KeelSshHandler`) over the
channel's byte stream. From then on it is spec 055's `contracts/runner.md`, byte for byte.

## Control subsystem `keel-relay-v1`

Frames are keel-exec `codec` frames (u32 big-endian length, JSON, ≤ 1 MiB), tagged `"t"`.
Unknown tags close the channel and the link.

| Frame | Direction | Fields | Notes |
|---|---|---|---|
| `Hello` | R → K | `version`, `relay_key_fingerprint`, `max_streams`, `http_cache: bool` | First frame; within 10 s. |
| `Ping` | K → R | `seq: u64` | Every health interval. |
| `Pong` | R → K | `seq`, `version`, `streams: u32`, `uptime_s: u64` | Within the interval, or the relay is `unreachable`. |
| `CacheGet` | R → K | `req: u32`, `release`, `path` | Only paths under `runner/<release>/` named in the release manifest (FR-010, research R7). |
| `CacheChunk` | K → R | `req`, `offset: u64`, `bytes` (base64, ≤ 512 KiB), `last: bool` | |
| `CacheMiss` | K → R | `req`, `reason` (`unavailable` \| `not_found` \| `too_large`) | `unavailable` until Keel serves `/runner` (055 T128). |
| `Close` | K → R | `reason` | Revocation or shutdown; the relay closes every runner stream. |

`CacheGet` is the only request a relay can make. Keel answers it from the release files it
serves to hosts directly, never from the database, the forge or the web interface (FR-004).
Keel rate-limits it to 4 concurrent requests and 64 MiB per release per relay.

## Relay behaviour

- **Runner listener**: accepts TCP on `runner_listen`. While no link is up, it accepts and
  closes at once. With a link, it opens one `forwarded-tcpip` channel and copies bytes both ways
  until either side closes. If Keel does not accept the channel within 20 s, the TCP connection
  is closed. No bytes are read, buffered beyond the channel window, or logged.
- **Limits**: `max_streams` (default 256); `max_pending_accepts` 64.
- **Release cache** (`http_listen`, default unset): `GET /runner/<release>/<path>` serves from
  `/var/lib/keel-relay/cache`, fetching through `CacheGet` on a miss. Plain HTTP on purpose:
  hosts check every file against the pinned SHA-256; the relay adds no trust (FR-010).
- **Logs**: link up/down, refused channels and requests, per-stream open/close with peer and
  byte counts. Never stream contents.
- **Config** `/etc/keel-relay/relay.toml` (`deny_unknown_fields`):

  ```toml
  link_listen = "0.0.0.0:7832"
  runner_listen = "0.0.0.0:7831"
  hub_keys = "/etc/keel-relay/hub_keys"      # one OpenSSH public line per key
  # http_listen = "0.0.0.0:7880"
  # max_streams = 256
  ```

- **CLI**: `keel-relay serve` (systemd), `keel-relay key` (prints exactly two lines, the public
  line then `SHA256:<fp>`, generating `/var/lib/keel-relay/host_ed25519` if missing), `keel-relay check-config` (reads `/etc/keel-relay/relay.toml` unless `--config` is given; exit 78 on an invalid file, as `serve` does).

## Runner additions (contracts/runner.md, additive)

1. **Dial list.** `--dial host:port` (repeatable) / `KEEL_RUNNER_DIAL` (comma-separated). The
   runner verifies the host certificate for `--server`'s host name whatever address it dialled.
   Without `--dial` behaviour is unchanged.
2. **`Hello.capabilities`** (optional, omitted by old runners): `["query-v1"]`.
3. **Queries** on `keel-runner-v1`:

   | Frame | Direction | Fields |
   |---|---|---|
   | `Query` | S → R | `query_id` (uuid), `kind`, `params` (object), `asked_by` (Keel user label) |
   | `QueryAnswer` | R → S | `query_id`, `answer` (object, ≤ 64 KiB serialised) |
   | `QueryRefused` | R → S | `query_id`, `reason`: `not_allowed` \| `bad_params` \| `unavailable` \| `too_large` \| `busy` |

   At most 4 queries in flight per session; a fifth is refused `busy`. A query never delays or
   blocks run frames.

   Answers by kind:

   | Kind | Params | Answer |
   |---|---|---|
   | `facts` | — | `hostname`, `os`, `kernel`, `arch`, `uptime_s`, `runner_version`, `ansible_core` |
   | `applied_revision` | — | `ledger_head`, `bases[]` (as in `Hello`) |
   | `unit_state` | `unit` | `load`, `active`, `sub`, `since` |
   | `disk_health` | — | per mount: `device`, `fs`, `read_only`, `errors` (from `/sys` and `/proc/mounts`); per disk: `smart_passed` when `smartctl` is installed |
   | `path_free` | `path` | `total_bytes`, `free_bytes`, `avail_bytes`, `inodes_free` |

4. **`/etc/keel-runner/trust/queries.toml`** (root-owned `0644`, `deny_unknown_fields`). Missing
   file: every query is refused `not_allowed`.

   ```toml
   kinds = ["facts", "applied_revision", "unit_state", "disk_health", "path_free"]
   units = ["k3s.service", "longhorn-*"]   # exact names, or a trailing * on a literal prefix
   paths = ["/var/lib/longhorn", "/"]      # exact paths; a query for a path below one is allowed
   ```

5. **Local query log** `/var/lib/keel-runner/queries.log`: JSON lines `{ at, query_id, kind,
   params, asked_by, outcome }`, rotated at 1 MiB keeping 3.

## Hostile-relay test matrix (SC-002)

Each must end with no frame acted on, no run started, an audit entry on Keel and the runner's
reconnect loop still running:

| Attack by the relay | Expected |
|---|---|
| flip one byte mid-session | inner SSH MAC failure, runner and Keel close the session |
| replay a recorded session's bytes on a new channel | inner key exchange fails (fresh ephemeral keys) |
| reorder or drop chunks | inner session fails integrity, closes |
| inject bytes before the runner's version banner | inner handshake fails |
| open a `forwarded-tcpip` channel with no runner behind it and attempt auth | gets `EnrollOnly` at best, enroll without a valid token is refused and audited |
| open `direct-tcpip`, `session`, `x11`; send `tcpip-forward`; send `exec` | refused, link dropped, relay quarantined, audited |
| present a host key other than the pinned one | handshake refused, relay quarantined, audited |
| send an unknown control frame or an oversized frame | link and every carried stream closed, relay quarantined, audited |
| `CacheGet` outside the manifest | `CacheMiss`, audited (not a violation: Keel sends `CacheGet`, the relay never does) |
| open 10 000 channels, or hold channels open without authenticating | refused beyond Keel's caps, idle ones closed after 10 s; Keel memory bounded; audit rows coalesced; direct runners unaffected |
| relay answers the runner with its own SSH server | runner refuses: host certificate does not verify for `--server`; no token or ledger bytes sent |
| serve a tampered release file over HTTP | host-side checksum fails (Ansible role), nothing installed |

## Keel never trusts the relay

Owner decisions of 2026-11-17 (tasks T059–T066). The relay host is treated as compromised.

- **Quarantine**: anything an honest relay never does takes it out of service, and only an owner
  can put it back. Keel drops the link and every carried stream, records `relays.quarantined_at`
  and its reason, and audits `relay.quarantine`. The supervisor stops dialling the relay until
  `PreviewRelayRelease` (workspace admin, step-up). Quarantine triggers:
  - a wrong host key;
  - a forbidden channel;
  - a global or channel request;
  - an unknown, oversized or out-of-order frame;
  - a second control channel.
- **Limits are enforced on Keel's side** whatever the relay claims:
  - at most 256 streams;
  - at most 20 new streams per second (burst 50);
  - 10 s from channel open to runner auth;
  - bounded russh windows and packets;
  - 1 MiB frames;
  - at most 4 unanswered pings.

  Hitting a limit refuses or closes a stream; it does not quarantine the relay.
- **Audit rows are coalesced** per relay and reason per minute, with a `count`.
- **The advisory peer** a relay reports is used for display and audit only, never for any
  security decision.
