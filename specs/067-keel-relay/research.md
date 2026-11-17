# Research: Keel relays (spec 067)

Decisions R1 and R2 resolve the spec's two `[NEEDS CLARIFICATION]` items. The owner confirmed
both on 2026-11-17. The rest follow from them and from the code as it stands on `6b94ebe1`.

## R1. Link transport: SSH over russh, Keel as client, relay as server

**Decision**: the relay link is one SSH connection that Keel dials to the relay. The relay runs
an embedded russh server; Keel runs a russh client. The algorithm lists are keel-exec's existing
ones (`server_preferred` / `client_preferred`: Ed25519 keys, `curve25519-sha256`,
`chacha20-poly1305@openssh.com` or `aes256-gcm@openssh.com`, public-key auth only, strict KEX).

- **Runner streams.** When a runner connects to the relay's local listener, the relay opens a
  **server-initiated `forwarded-tcpip` channel** to Keel (russh server `Handle::
  channel_open_forwarded_tcpip`). The channel's "connected address" is the fixed string
  `keel-runner` and port `0`; the originator address and port are the runner's TCP peer, which
  Keel records as advisory only (the relay can lie about it). Keel's client handler accepts that
  channel type and nothing else, and hands the `ChannelStream` to the same
  `russh::server::run_stream(config, stream, KeelSshHandler)` the TCP listener uses
  (`apps/keel/server/src/runner_listener.rs:612`). The runner's SSH session runs inside the
  channel, untouched (FR-002, FR-003).
- **Control.** Keel opens one session channel on the link with the subsystem
  `keel-relay-v1`. It carries length-prefixed JSON frames (keel-exec's `codec`): health pings,
  relay status, and cache requests (contracts/relay.md). Health and cache share one subsystem
  so that the relay's policy is "one control channel, plus forwarded-tcpip channels it opens
  itself", which is easy to state and to test.
- **Refusals.** Keel's client handler refuses every channel type the relay opens except
  `forwarded-tcpip` to `keel-runner:0`, and refuses `direct-tcpip`, X11, agent forwarding,
  global requests and `exec`/`shell`/`pty`/`env`. The relay's server handler refuses everything
  but one session channel with the `keel-relay-v1` subsystem from the pinned Keel key (FR-004).

**Rationale**:

- russh 0.65 is already in the server and runner, and keel-exec already pins algorithms and
  has the handler pattern (`KeelSshHandler`). The relay is "another `KeelSshHandler`-shaped
  policy" rather than a new stack.
- `forwarded-tcpip` is the SSH-standard way for a server to push a stream to a client. Keel never
  binds a port or listens on the relay; the relay cannot ask Keel to connect anywhere.
- Keepalives, flow control (channel windows) and integrity are SSH's. A hostile relay that
  alters a byte of a runner stream breaks the inner session's AEAD, so the inner session closes
  (Story 2 scenario 1) and nothing in the relay link has to detect it.
- The hostile-server test (`apps/keel/runner/tests/hostile.rs`) already drives a real russh
  session in-process; a hostile relay is the same harness with a byte-tampering hop.

**Alternatives considered**:

- **TLS + yamux (or h2).** rustls is in the tree, but it would add a mux crate, a second identity
  and pinning scheme beside the SSH keys, and hostile-peer tests from scratch. Rejected by the
  owner.
- **Relay as an SSH `direct-tcpip` jump host** (Keel opens `direct-tcpip` towards runners).
  That reverses who opens streams: the relay would have to hold runner connections until Keel
  asked for them, and `direct-tcpip` is exactly the "general tunnel" FR-004 forbids. Rejected.
- **One link per runner.** Keel would dial the relay once per runner and lose the single health
  view of the link. Rejected.

## R2. Registration: paste keys both ways, nothing trusted on first use

**Decision**:

- **Relay identity.** On its first start `keel-relay` generates an Ed25519 host key
  (`/var/lib/keel-relay/host_ed25519`, `0600`, owner `keel-relay`) and prints its OpenSSH public
  line and SHA-256 fingerprint (also `keel-relay key` at any time). In Settings → Relays the
  owner enters the relay's name, zone and dial address and pastes that public line. Preview shows
  the fingerprint; confirm stores it and audits the change. Keel's link client accepts only that
  exact host key for that relay (plain key, not a certificate: there is no relay CA).
- **Keel identity.** Keel keeps one **relay-link key**, an Ed25519 key generated on first start,
  sealed under the master key in the database like the SSH CAs (`hosts.rs`, aad
  `relay_link_key:<id>`). Settings → Relays shows its public line, ready to paste into the
  operator's trust file as `keel_relay_hub_keys` (a list, so a rotation can pin old and new key
  together). The relay accepts a link only from a key in that list (FR-001).
- **Rotation.** Keel's relay-link key rotates the way SSH CAs do: create a new key (it becomes
  pending), add it to `keel_relay_hub_keys`, activate, remove the old one. A relay host key is
  rotated by re-pasting (edit, preview, confirm). Both are audited.

**Rationale**: Keel dials the relay, so the relay never presents anything to Keel before Keel has
already authenticated to it. A token would have to travel the wrong way, and the first contact
would be trust-on-first-use from one side. Pasting keys costs the owner two copies and leaves no
window where an unpinned peer is accepted.

**Alternatives considered**: a one-time token in the relay's configuration that Keel proves on
first dial, after which both sides pin keys. More code, a TOFU window, and nothing gained for a
handful of relays. Rejected by the owner.

Note on the spec's variable name: FR-008 and the spec use singular forms. The trust file variable
is `keel_relay_hub_keys` (list) for rotation; the role fails on an empty list.

## R3. How Keel serves relayed sessions without changing the runner contract

**Decision**: the runner listener's per-connection code is factored so the same handler runs over
a TCP socket or over a relay channel, with a `RunnerPath` value (`Direct { peer }` or
`Relay { relay_id, advisory_peer }`) attached to the connection. `queue::attach` already takes any
`AsyncRead + AsyncWrite` (`queue.rs:573`). The path goes into the run hub's session state, into
`hosts.last_path` and into the audit entries the listener already writes (FR-005).

The runner verifies Keel's host certificate for the name in `keel_runner_server`, wherever it
dialled (R4). Keel's host certificate is already issued for `--runner-host-names`; nothing changes
there. The relay never holds a Keel host key or certificate, so it cannot answer for Keel.

## R4. Runner dial list

**Decision**: `keel-runner` gains `--dial <host:port>` (repeatable; env `KEEL_RUNNER_DIAL`,
comma-separated). `--server` stays the verified name (`HostCertVerifier::new(cas, host)`); with
no `--dial`, the runner dials `--server` as today. With a dial list it tries each address in order,
per attempt, and uses the first that completes the SSH handshake **with a host certificate that
verifies for `--server`'s host name**. A dial address that answers with anything else counts as a
failed attempt, is logged with the address, and the next is tried. Backoff is the existing
reconnect backoff, applied per full pass over the list (default ceiling 60 s, SC-003).

The runner reports nothing new about its path: Keel knows it from where the stream arrived.

**Alternatives considered**: a per-address "expected name" (rejected: there is one Keel; the
relay must not be a name a runner trusts); SRV records (rejected: no DNS dependency on the spoke).

## R5. Zones

**Decision**: a zone is a row per workspace (`zones`), hosts get a nullable `zone_id`, relays get
a required `zone_id`. A host with no zone is in the implicit `direct` zone, which is not stored.
Zone membership of a host is set from the Hosts page (preview and confirm, audited) and is
advisory: it decides grouping and the "only relay of a zone" warning, not which path a runner
uses. The path is whatever the runner actually came in on.

"No path" (spec edge case) is shown for a host in a zone whose relays are all unreachable or
revoked, and which is not connected.

## R6. Health checks

**Decision**: Keel's relay supervisor keeps one task per non-revoked relay. It dials, sends a
`Ping` on the control channel every `--relay-health-interval` (default 30 s, SC-003), and marks
the relay `unreachable` after one missed interval (no answer within the interval, or the link
dropped and a redial failed). Reconnect uses exponential backoff capped at the health interval.
Each `Pong` carries the relay's version and the number of runner streams it currently carries;
Keel measures round-trip time itself. State changes (not every ping) are audited and pushed to the
web UI as an event.

## R7. Caching (FR-010) is narrowed for this spec

FR-010 is a MAY. Two findings shape it:

- **Repository content never leaves the runner session.** Keel sends commits to runners as `Pack`
  frames inside `keel-runner-v1` (contracts/runner.md). The relay cannot see those bytes, so it
  cannot cache them, and changing that would mean a second, out-of-session fetch path. **Decision:
  no git-object caching in 067.** The edge case "relay serves a stale cache of the repository"
  cannot arise. Revisit if packs over a slow link become a measured problem.
- **Runner release files** are fetched by the Ansible role from `<origin>/runner/<release>/...`
  (`../ansible/roles/keel_runner/tasks/binaries.yml:29`), but Keel has no `/runner` route yet
  (055 T128/T147/T149). **Decision: the relay serves release files** on a local HTTP listener
  (`keel_relay_http_listen`, default off) at the same paths, fetched once over the control channel
  (`CacheGet { release, path }` → chunks) and kept under `/var/lib/keel-relay/cache/<release>/`.
  Hosts point `keel_runner_download_origin` at the relay and verify every file against
  `keel_runner_sha256` exactly as today. This phase is built last and is blocked on the 055 route;
  until then `CacheGet` answers `unavailable`.

## R8. Typed queries (Story 5)

**Decision**:

- **Frames** (additive to `keel-runner-v1`, contracts/relay.md §Queries): `Query { query_id,
  kind, params, asked_by }` S → R and `QueryAnswer { query_id, answer }` /
  `QueryRefused { query_id, reason }` R → S. Kinds: `facts`, `applied_revision`, `unit_state
  { unit }`, `disk_health`, `path_free { path }`.
- **Capability.** `Hello` gains an optional `capabilities: Vec<String>` (omitted by old runners).
  Keel sends `Query` only to runners that list `query-v1`; for others the UI says the runner is
  too old. This keeps contracts/runner.md backward compatible, and an old runner never sees an
  unknown frame.
- **Allow list in its own file**: `/etc/keel-runner/trust/queries.toml`. Not in `policy.toml`,
  because `policy.toml` is `deny_unknown_fields` and fails whole: an older runner reading a new
  key would stop running jobs. A missing `queries.toml` refuses every query.
- **No shell.** Unit state is read with `systemctl show --property=LoadState,ActiveState,
  SubState,ActiveEnterTimestamp -- <unit>` through `Command` with an argv array, after the unit
  name matches the allow list exactly and `^[A-Za-z0-9@._:-]+$`. Free
  space is `statvfs` on an allowed path. Disk health reads `/proc/mounts` and
  `/sys/block/*/stat` plus `smartctl --json` with an argv array when installed. Facts and applied
  revision come from what the runner and executor already hold (`Hello` data, ledger head). The
  network-side process answers what it can unprivileged; `disk_health` (SMART) goes to the
  executor over its socket, like `ReadLedger`.
- **Audit.** Keel audits every query and its outcome with the asking user. The runner appends
  each query to a local log (`/var/lib/keel-runner/queries.log`, JSON lines, rotated by size)
  with the `asked_by` Keel sent. The host cannot verify `asked_by`; the log records what Keel
  claimed, which is what FR-014 asks for.

Not D-Bus: `zbus` is in the lockfile only through the desktop app's keyring
(`zbus-secret-service-keyring-store`), and the runner should not gain it for one property read.
`systemctl show` with an argv array is as safe and adds nothing.

## R9. Relay process model and packaging

**Decision**: one binary `keel-relay` in a new crate `apps/keel/relay` (lib + bin, like
`apps/keel/runner`), unprivileged system user `keel-relay`, systemd unit with
`NoNewPrivileges`, `ProtectSystem=strict`, `StateDirectory=keel-relay`, no capabilities (ports
above 1024). Config in `/etc/keel-relay/relay.toml` (`deny_unknown_fields`): `link_listen`
(default `0.0.0.0:7832`), `runner_listen` (default `0.0.0.0:7831`), `hub_keys` (file
`/etc/keel-relay/hub_keys`), optional `http_listen`, limits. The link policy types (control
frames, handler refusals) live in `crates/keel-exec/src/relay.rs` so the server and the relay
share one definition and one round-trip test.

The release that ships `keel-runner` ships `keel-relay` (same build, same checksums file),
pinned in the trust file as `keel_relay_sha256` (FR-015).

Limits, so a relay host cannot be pushed over: at most 256 concurrent runner streams (config),
runner TCP connections that Keel does not take within 20 s are closed, and while the link is down
the relay accepts and immediately closes runner connections (so runners back off; the relay keeps
no state about them, Story 1 scenario 4).

## R10. Revocation and "only relay" warning

- Revoking a relay sets `revoked_at`, cancels its supervisor task (closing the link and every
  runner stream on it), and the supervisor refuses to start for a revoked row. Runs in progress on
  those streams are handled as lost runner connections (spec 055), so are never run twice
  (Story 4 scenario 2).
- Run preview gains a warning when a job's targets include a host that is the only non-revoked
  relay of a zone and other targets are in that zone. The relay host is matched by
  `relays.host_id` (optional link from a relay to the Keel host it runs on, set at registration).

## R11. Scope of the Ansible changes

The operator's repository is `../ansible`, outside this repository. As in spec 055
(contracts/host-install.md), Keel documents what it relies on in `contracts/ansible.md`; the
`keel_relay` role, the `keel-relay` profile and the `keel_runner_dial` variable are written there
with the owner's go-ahead, as separate tasks.
