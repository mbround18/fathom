---
description: "Task list for spec 067, Keel relays for hosts Keel cannot be reached from"
---

# Tasks: Keel relays for hosts Keel cannot be reached from

**Paths**:

- `server/` means `apps/keel/server/`.
- `runner/` means `apps/keel/runner/`.
- `relay/` means `apps/keel/relay/` (new).
- `web/` means `apps/keel/web/`.
- `exec/` means `crates/keel-exec/src/`.
- `proto/` means `crates/keel-protocol/src/`.

**Gates for agents**:

- Run only targeted checks for what you touched, one command at a time, with
  `CARGO_BUILD_JOBS=4`, logging to files. For example `cargo test -p keel-exec relay`,
  `cargo test -p keel-server --test relay_link`, `pnpm --filter @companion/keel-web test`, or
  one Playwright project (`--project=s067`).
- **Never** run `scripts/ci.sh`, `paws`, `dagger` or the full Playwright suite. The main session
  runs the full gate once, on the combined branch.
- Server integration tests need `KEEL_TEST_DATABASE_URL`. Report the `running N tests` count for
  each test file you ran, so a silent skip is visible (plan D4).
- Delete any kind cluster or compose project you start before reporting.

**Tests**: the spec asks for them (Independent Tests, SC-002), and constitution Principle III
requires them. Write the tests in each story first, and see them fail before you implement.

**External work**: tasks marked **(../ansible, owner go-ahead)** change the operator's
repository, outside this one. Do not start them until the owner says go.

## Phase 0: Blockers (plan D1–D4; the owner gave the go-ahead on 2026-11-17)

Migration numbers are fixed so that parallel branches don't collide: `0015_join.sql` (D3),
`0016_run_host_lock.sql` (D2), `0017_relays.sql` (this spec).

- [X] T055 D2: the per-host run lock in `server/src/queue.rs` `dispatch_ready`. A host with a
  `run_hosts` row in `sent` or `running` gets no new assignment; later runs stay queued and
  dispatch in `created_at` order. Backstop: a partial unique index on `run_hosts (host_id) where
  state in ('sent','running')` in `server/migrations/0016_run_host_lock.sql`. Test it in
  `server/tests/runs.rs`.
- [X] T056 D4: `server/tests/common/mod.rs` `TestDb::new` panics when `CI` is set and
  `KEEL_TEST_DATABASE_URL` is unset. `scripts/ci.sh` starts `scripts/keel-test-db.sh` and
  exports the URL for the keel-server tests. Fix any test this exposes.
- [X] T057 D1: spec 055 T128, T140, T147 and T149: the release build target, the
  `/runner/…` routes from `--runner-dist-dir`, and the image's `runner-dist` stage.
- [X] T058 D3: spec 055 T137–T146 and T148: join tokens end to end.

## Phase 1: Setup

- [X] T001 Create crate `relay/` (lib + bin `keel-relay`) with `Cargo.toml` on the workspace's
  `russh = "0.65.0"`, tokio, serde, toml, clap and tracing, and empty modules
  `src/{lib,config,link,forward,cache,key}.rs` and `src/bin/keel-relay.rs`. Add it to the
  workspace `members` in `Cargo.toml`, and check `cargo build -p keel-relay`.
- [X] T002 [P] Add Playwright project `s067` (`testDir: './tests/e2e/s067'`) to
  `web/playwright.config.ts`, update its header comment list, and create
  `web/tests/e2e/s067/` and an empty `web/tests/support/fake-relays.ts`.
- [X] T003 [P] **Superseded by T056** (`TestDb::new` panics under CI). Was: add `server/tests/common` helper `require_db()`. Under `CI=1` it panics when
  `KEEL_TEST_DATABASE_URL` is unset; otherwise it prints one `SKIPPED: KEEL_TEST_DATABASE_URL
  unset` line to stderr. Use it in every new server test in this spec (plan D4). Do not change
  the 19 existing files.

## Phase 2: Foundational (blocks every story)

- [X] T004 Write `server/migrations/0017_relays.sql` per data-model.md:
  - tables `zones`, `relay_link_keys`, `relays`, `host_queries`;
  - `hosts.zone_id` and `hosts.last_path jsonb`;
  - zone/relay name check `^[a-z0-9][a-z0-9-]{0,62}$`, with `direct` reserved;
  - unique `(workspace_id, name)` on zones and relays, and unique `host_key_fingerprint`;
  - one `active` relay-link key (partial unique index);
  - grants of `select, insert, update` to `keel_app`, with `delete` only on `host_queries`.

  Extend `server/tests/migrate.rs` to apply it.
- [X] T005 [P] Test first, `exec/relay.rs` tests:
  - serde round trips for every control frame in contracts/relay.md (`Hello`, `Ping`, `Pong`,
    `CacheGet`, `CacheChunk`, `CacheMiss`, `Close`);
  - an unknown tag fails;
  - a frame over 1 MiB fails.
- [X] T006 Implement `exec/relay.rs` and export it from `exec/lib.rs`:
  - the control `Frame` enum on keel-exec `codec`;
  - `RELAY_SUBSYSTEM = "keel-relay-v1"`, `LINK_USER = "keel"`, and the
    `RUNNER_CHANNEL_ADDR = ("keel-runner", 0)` constants;
  - `relay_server_config(host_key)`, a russh server `Config` built from `server_preferred()`
    with no certificate;
  - `link_client_config()` from `client_preferred()`, with a 15 s keepalive.
- [X] T007 [P] Test first, `exec/relay.rs` policy tests over an in-process russh pair.
  `RelayServerPolicy` accepts only user `keel` with a key in `hub_keys`, and only one `session`
  channel with subsystem `keel-relay-v1`. It refuses `direct-tcpip`, `x11`, agent forwarding,
  `tcpip-forward`, `exec`, `shell`, `pty`, `env` and a second session. `LinkClientPolicy`
  accepts only `forwarded-tcpip` to `keel-runner:0` and only the pinned relay host key.
- [X] T008 Implement `RelayServerPolicy` and `LinkClientPolicy` (russh `server::Handler` and
  `client::Handler`) in `exec/relay.rs`. Each refusal is reported through a callback, so the
  server can audit it.
- [X] T009 [P] Add `Hello.capabilities: Vec<String>` to `exec/frames.rs`
  (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`). Add a round-trip test, and a test
  that an old `Hello` without the field still parses.
- [X] T010 Thread a `RunnerPath { Direct { peer }, Relay { relay_id, advisory_peer } }` through
  `server/src/runner_listener.rs`. Factor the per-connection code into
  `serve_connection(app, config, stream, path)`, so it accepts any
  `AsyncRead + AsyncWrite + Unpin + Send` stream. The TCP accept loop then calls it with
  `Direct`. Pass the path to `queue::attach` (`server/src/queue.rs:573`). Store it in the run
  hub's session entry, and write `hosts.last_path` on `Hello`. The existing connect and
  disconnect audit entries gain `path`. Existing runner tests must stay green
  (`cargo test -p keel-server --test enroll --test runs`).
- [X] T011 Add Keel's relay-link key in `server/src/relays.rs`, following the `ssh_cas` code in
  `server/src/hosts.rs`:
  - generate an Ed25519 key on startup when no `active` row exists;
  - seal it under the master key with aad `relay_link_key:<id>`;
  - load it into `AppState`;
  - audit `relay_link_key.create`.
- [X] T012 Add server config `--relay-health-interval` / `KEEL_RELAY_HEALTH_INTERVAL` (default
  30 s) and runtime flag `KEEL_RELAYS` (default on) in `server/src/config.rs`, with tests next
  to the existing `runner_listen` config tests.
- [X] T013 [P] Implement `relay/src/config.rs`: `relay.toml` with `deny_unknown_fields` and the
  keys `link_listen` (default `0.0.0.0:7832`), `runner_listen` (default `0.0.0.0:7831`),
  `hub_keys` (path), `http_listen` (optional) and `max_streams` (default 256). Implement
  `relay/src/key.rs`: generate `/var/lib/keel-relay/host_ed25519` with mode `0600` when missing,
  and print the public line and `SHA256:` fingerprint. A `hub_keys` file that is empty or holds
  a non-`ssh-ed25519` line fails closed. Add unit tests for each.

**Checkpoint**: migration applies, the link policy is tested in isolation, and the runner
listener can serve a non-TCP stream.

## Phase 3: User Story 1, run jobs on a host Keel cannot be reached from (P1) 🎯 MVP

**Goal**: a runner that can only reach a relay connects to Keel through it and runs jobs exactly
as a direct host does.

**Independent test**: `cargo test -p keel-server --test relay_link`. Keel binds no runner
listener and dials an in-process relay. A real runner, dialling only the relay, enrolls, shows
as `connected · via relay`, and runs a fixture job with its log streamed. Stopping the relay
leaves the host disconnected, not failed.

### Tests (write first)

- [X] T014 [P] [US1] `server/tests/relay_link.rs` covers US1 acceptance 1–5:
  - path recorded as the relay;
  - a run streams its logs and its result;
  - the runner refuses a relay that answers with its own key instead of Keel's host
    certificate;
  - with Keel unable to reach the relay, the runner keeps retrying and the host shows
    disconnected;
  - enrollment through the relay works, and the token never appears in the relay's logs or
    buffers (assert on the relay's captured tracing output).
- [X] T015 [P] [US1] `runner/tests/dial.rs`:
  - dials in order;
  - skips a dialled address whose host cert does not verify for `--server`'s name;
  - with no `--dial`, dials `--server` as before;
  - backoff applies per full pass and is capped at 60 s.
- [X] T016 [P] [US1] `relay/tests/link.rs`:
  - with no link, a runner TCP connection is accepted and closed at once;
  - with a link, bytes are copied both ways unchanged;
  - a channel Keel does not accept within 20 s closes the TCP side;
  - stream number `max_streams + 1` is refused.

### Implementation

- [X] T017 [US1] Implement `relay/src/link.rs`. It is an SSH server on `link_listen` using
  `RelayServerPolicy`, and allows one link at a time: a new authenticated link replaces the old
  one and closes its streams. It serves the `keel-relay-v1` control channel: `Hello` first, then
  `Pong` for each `Ping` with `streams` and `uptime_s`. `CacheGet` is not sent yet.
- [X] T018 [US1] Implement `relay/src/forward.rs`, the runner TCP listener on `runner_listen`.
  For each connection it opens a `forwarded-tcpip` channel `keel-runner:0`, with the runner's
  peer as originator, and runs `copy_bidirectional`. It enforces `max_streams` (256) and
  `max_pending_accepts` (64). It logs open and close with peer and byte counts, and never logs
  contents.
- [X] T019 [US1] Implement `relay/src/bin/keel-relay.rs` with the subcommands `serve`, `key` and
  `check-config`, and `relay/systemd/keel-relay.service` (matching `runner/systemd/`; ../ansible byte-copies it) with `RestartPreventExitStatus=78`, `ReadOnlyPaths=/etc/keel-relay`, the runner unit's hardening, and runs as an unprivileged `keel-relay` user
  with `NoNewPrivileges`, `ProtectSystem=strict`, `StateDirectory=keel-relay` and no
  capabilities.
  - Done 2026-11-17: `keel-relay` tests 12 in `tests/link.rs`, 4 in `tests/cli.rs`, 11 unit,
    1 doc. The unit already carries T065's sandbox list;
    `systemd-analyze security --offline=true keel-relay.service` scores **1.1 OK** (remaining
    exposure: Internet sockets, host network, no IP allow list, host root, `char-rtc`).
- [X] T020 [US1] Implement `RelaySupervisor` in `server/src/relays.rs`:
  - one task per non-revoked relay row;
  - dial `dial_address` with `link_client_config()` and `LinkClientPolicy`, pinned to
    `relays.host_key`;
  - authenticate as `keel` with the active relay-link key;
  - open the control channel, then `Ping` each health interval and measure RTT;
  - keep `RelayLive { state, rtt_ms, version, streams, last_error, since }` in memory;
  - redial with exponential backoff capped at the health interval;
  - audit state changes as `relay.state`;
  - start the tasks from `server/src/serve.rs` when `KEEL_RELAYS` is on.
- [X] T021 [US1] On each accepted `forwarded-tcpip` channel in `server/src/relays.rs`, call
  `runner_listener::serve_connection` with `RunnerPath::Relay { relay_id, advisory_peer }` over
  the channel stream. Keep a handle per stream, so revocation (T027) can close them.
- [X] T022 [US1] Add the runner dial list in `runner/src/runner/mod.rs`:
  - `RunnerConfig.dial: Vec<String>`, with `--dial` (repeatable) and `KEEL_RUNNER_DIAL`
    (comma-separated) in `runner/src/bin/keel-runner.rs`;
  - `connect()` tries each address in order, building `HostCertVerifier` from `--server`'s host
    every time;
  - the existing backoff loop (`mod.rs:533`) wraps a full pass.
- [X] T023 [US1] Add a minimal relay registration path (needed by T014 before US3's UI): a
  server function `register_relay(tx, workspace, zone, name, dial_address, host_key, host)` in
  `server/src/relays.rs`. It validates `dial_address` (`host:port`, port 1–65535) and `host_key`
  (exactly one `ssh-ed25519` line), and audits `relay.register`. US3 wraps it in
  preview/confirm.

**Checkpoint**: the MVP works end to end in-process. Run T014–T016 and record the test counts.

## Phase 4: User Story 2, a relay cannot be used to attack hosts or Keel (P1)

**Goal**: whoever controls the relay host can drop or delay traffic, and nothing more.

**Independent test**: the hostile-relay suites run the SC-002 matrix in contracts/relay.md. Every
attempt fails closed, no job runs, and each attempt leaves an audit row.

- [X] T024 [P] [US2] `runner/tests/hostile_relay.rs`, built on `runner/tests/hostile.rs`'s
  harness with a tampering hop between runner and server. It covers flip a byte, replay a
  recorded session, reorder chunks, drop chunks, and inject before the banner. Each must close
  the session with no run directory, an unchanged ledger, and the reconnect loop still running.
- [ ] T025 [P] [US2] `server/tests/hostile_relay.rs` uses a hostile in-process relay that:
  - opens `forwarded-tcpip` with no runner behind it and tries auth (gets enroll-only, and is
    refused without a token);
  - opens `direct-tcpip`, `session` and `x11`;
  - sends `tcpip-forward` and `exec`;
  - presents a host key other than the pinned one;
  - sends an unknown control frame, then an oversized one.

  Assert a `relay.refused` audit row for each, and no run.

  Remaining: `tcpip-forward` and `exec` are refused by russh 0.65 itself with no handler hook, so
  the test asserts the refusal but there is no audit row; needs an upstream russh hook or a patch.
- [X] T026 [US2] Wire the refusal callbacks from T008 into `server/src/relays.rs`. Every refusal
  writes `relay.refused { reason, detail }` (coalesced per T061). A **protocol violation**
  quarantines the relay (T059); a refused runner auth on a carried stream does not, because that
  is a runner's failure, not the relay's.
- [X] T027 [US2] Implement revocation in `server/src/relays.rs`, US2 acceptance 6 and FR-006:
  - `revoke_relay` sets `revoked_at` and audits `relay.revoke` in one transaction;
  - it sends `Close`, drops the link, and closes every carried runner stream at once;
  - runs on those streams follow the lost-runner path in `queue.rs`;
  - the supervisor never dials a revoked row.

  Test it in `server/tests/relays.rs`.

### Hardening: Keel never trusts the relay (owner request, 2026-11-17)

The relay host is assumed compromised. Every limit the relay enforces is enforced again on Keel's
side, and anything an honest relay never does takes the relay out of service.

- [ ] T059 [US2] **Quarantine** (owner decision). These are protocol violations an honest relay
  never commits:
  - a host key other than the pinned one;
  - a channel other than `forwarded-tcpip` to `keel-runner:0`;
  - any global or channel request (`tcpip-forward`, `exec`, `shell`, `pty`, `env`, agent, x11);
  - an unknown, oversized or out-of-order control frame;
  - a second control channel.

  On any of them, Keel at once drops the link and closes every stream the relay carries. In the
  same transaction it sets `relays.quarantined_at` and `quarantine_reason`, and audits
  `relay.quarantine`. The supervisor never dials a quarantined relay. `PreviewRelayRelease`
  (workspace admin, step-up) clears it; its preview shows the reason, the time and the refusals
  since. Before a carried stream is served, Keel re-reads the relay row, so a stream that races a
  revoke or quarantine is refused. Test it in `server/tests/hostile_relay.rs`, checking every
  violation above.

  Remaining: global requests and unknown channel requests (`tcpip-forward`, `exec`, `shell`,
  `pty`, `env`, agent) are refused by russh without a hook, so they do not quarantine yet. Owner
  decision 2026-11-17: accept refuse-only (no russh fork); document the gap in the threat model
  and raise an upstream russh issue/PR for a client-side request hook, adopted once released; the
  `PreviewRelayRelease` mutation and preview are US3 (`release_relay` is in place for it).
- [ ] T060 [US2] **Keel-side limits, independent of the relay's own**, in `server/src/relays.rs`
  and `exec/relay.rs`. A server config value sets each:
  - at most 256 concurrent carried streams per relay, beyond which channel opens are refused;
  - at most 20 new streams per second per relay (token bucket, burst 50);
  - a 10 s limit from channel open to the runner's SSH authentication, after which the stream
    closes;
  - russh client `Config` with a bounded `window_size` and `maximum_packet_size`, an
    `inactivity_timeout`, and keepalives;
  - control frames capped at 1 MiB, with at most 4 unanswered `Ping`s.

  Exceeding a limit refuses the stream or closes it and is audited (coalesced). It does not
  quarantine the relay, since a busy honest zone can hit these limits. Test with a flooding
  relay: Keel stays within a fixed memory bound, and direct runners and other relays stay
  served.

  Remaining: the 1 MiB frame cap, 4 unanswered Pings, link window/packet sizes and keepalives
  are constants in `keel-exec`, not server config values (streams, rate, burst and auth deadline
  are config).
- [X] T061 [US2] **Audit and log flood control**: `relay.refused` and limit hits are coalesced
  per `(relay, reason)` per minute into one row with a `count`. `tracing` output on relay paths
  is rate-limited the same way. A relay can never grow the audit table faster than this. Test
  that 10 000 refusals in a minute give one row with `count = 10000`.
- [X] T062 [US2] **The advisory peer is never trusted**. A relay's claimed `advisory_peer` is used
  only for display and audit. No authentication, rate-limit key, allow list or host identity is
  derived from it. Enrollment and join attempts through a relay are rate-limited per relay id.
  Add a test, and a grep-style unit test that `advisory_peer` is read only by the
  display/audit functions.

  Enrollments use the per-relay bucket in `LiveRelays::attempt` (`KEEL_RELAY_ENROLL_*`); joins
  are owned by `join::JoinLimiter`, keyed `relay:<id>` (`KEEL_JOIN_RATE`), and a relayed join it
  refuses is a coalesced `relay.refused { reason: join_rate }`.
- [X] T063 [US2] **Runner fails closed through a relay**, in `runner/tests/hostile_relay.rs`. A
  relay that answers with its own SSH server, or replays Keel's host certificate with a
  different key, gets nothing:
  - no enrollment token, join token or ledger bytes leave the runner before Keel's host
    certificate verifies for `--server`'s name;
  - an algorithm downgrade (anything outside keel-exec `server_preferred()`) is refused.
  *The runner has no join token yet; when it lands it must go through the same `connect()`
  path, which these tests cover with the enrollment token.*
- [X] T064 [P] [US2] **Property tests** with `proptest`, a dev-dependency only (owner decision;
  check with `cargo deny`). Arbitrary bytes into the control-frame decoder, `Hello` and the
  query frames, `relay.toml`, `hub_keys` and `queries.toml` parsing. None may panic,
  allocate more than the frame cap, or accept anything the strict parser would refuse. Put them
  in `exec/relay.rs` tests, `relay/tests/` and `runner/tests/queries.rs`.
  *Done for the control frames, `Hello`, `relay.toml` and `hub_keys` (`keel-exec/tests/relay_props.rs`,
  `relay/tests/props.rs`). The query frames and `queries.toml` are still owed: add them with US5.*
- [X] T065 [US2] **Relay sandbox** in `relay/systemd/keel-relay.service`:
  - the runner unit's hardening, plus `CapabilityBoundingSet=` and `AmbientCapabilities=`
    (empty);
  - `SystemCallFilter=@system-service ~@privileged ~@resources`,
    `RestrictAddressFamilies=AF_INET AF_INET6`, `MemoryDenyWriteExecute=yes`;
  - `MemoryMax=256M`, `TasksMax=64` and `LimitNOFILE=1024`, with `ReadOnlyPaths=/etc/keel-relay`
    and `StateDirectory=keel-relay`.

  Record the `systemd-analyze security keel-relay.service` score (target ≤ 2.0 "OK") in this file.
  Done 2026-11-17: every directive above is present (a8c3781e); offline score 1.1 OK; fluxified told the same day.
  Tell the fluxified session when the unit changes, because ../ansible byte-copies it.
- [X] T066 [US2] **Independent adversarial review**, before US3 merges. A fresh agent gets only
  the code, `contracts/relay.md` and this hardening list, with none of the implementers'
  conclusions, and is told to break it, either as a hostile relay or as an attacker on the
  link. Every finding is fixed or recorded with the owner's sign-off. Then run the
  `security-review` skill over the branch diff.
  Findings (PoCs in `apps/keel/server/tests/t066_review.rs`, all passing):
  - **F1 fixed**: the stream cap, rate and "control channel first" rule now apply at
    `forwarded-tcpip` open (`StreamAdmission`, refused as `ResourceShortage`). Control writes go
    through a bounded queue to a writer task. A stalled write or a full queue ends the link as
    lost with backoff, not as a violation: a stalled network looks the same as a relay that
    stopped reading.
  - **F2 fixed**: `JoinLimiter` is checked before the first join frame is read. Enroll and join
    first frames are capped at 64 KiB.
  - **F3 fixed**: only Runner and RenewOnly lift the relay auth deadline. Enroll and join
    sessions are bounded by `NON_RUNNER_LIFETIME` (60 s), and every session ends with its
    subsystem.
  - **F4 fixed**: a relay carries runners, renewals, enrollments and joins of its own workspace
    only. Anything else is refused as `relay.refused` with reason `workspace_mismatch`.
    Enrollment tokens are not burned. There is no zone check. See `contracts/relay.md`.
  - **F5 fixed**: at most 4 unanswered `CacheGet`s per relay; more are refused as
    `cache_get_busy`. A release over 64 or a path over 256 bytes gets `NotFound`. The logged
    refusal detail is capped.
  - **F6 fixed**: a quarantine is held in memory until its database write lands. `sync` honours
    it, so the relay is not redialled, and retries the write.
  - **F7 not changed**, by direction. It is an **open owner question**. Anyone on the network
    path who presents a wrong host key quarantines the relay until a human releases it, which is
    availability abuse by design. Options: rate-limit host-key quarantines, or quarantine only when
    a mismatch follows authenticated data (and treat a pre-auth mismatch as a lost link).
  - **Relay binary**:
    - A carried stream idle for `LINK_INACTIVITY` (120 s) is now closed.
    - "A newer link replaces the old one" is unchanged. Only a key in `hub_keys`, so Keel
      itself, can open a link, and a fresh link is how Keel recovers from a half-dead one.
- [X] T067 Drop OpenSSL from the relay and runner binaries (owner decision 2026-11-17: rustls
  only). Today `keel-trust` → `web-passkeys` → `webauthn-rs-core` → `webauthn-attestation-ca` →
  `openssl` puts OpenSSL in `keel-exec`, `keel-relay` and `keel-runner` (`cargo tree -i
  openssl-sys`). Split what `keel-trust` needs from `web-passkeys` (assertion verification only)
  behind a feature or a small crate with no webauthn-rs attestation dependency, so `cargo tree -i
  openssl-sys -p keel-relay -p keel-runner -e normal` is empty; the server may keep it. Gate:
  keel-trust, keel-exec, keel-runner, keel-relay tests and clippy.
  Done: `web-passkeys` gained a default `ceremonies` feature (webauthn-rs, store, recovery);
  `keel-trust` takes it with `default-features = false`, so the `cargo tree` above is empty.

## Phase 5: User Story 3, see zones, relays and how each host is reached (P1)

**Goal**: the Hosts page groups hosts by zone and shows each host's path, and Settings → Relays
manages relays and Keel's link key.

**Independent test**: with two zones, one direct and one behind a relay, stopping the relay turns
only that zone's hosts to `disconnected (relay unreachable)`, and the change is audited (Vitest,
Playwright `s067`, `server/tests/relays.rs`).

- [X] T028 [P] [US3] Add the protocol types from contracts/protocol.md in a new `proto/relay.rs`:
  `ZoneInfo`, `RelayState`, `RelayInfo`, `RunnerPath`, `HostReach`, `RelayLinkKey`, `QueryKind`,
  `HostQuery`.
  - `HostInfo` gains `zone: Option<Uuid>` and `reach: HostReach`, both `#[serde(default)]`.
  - Add the commands under a "Zones and relays" section in `proto/cmd.rs`, with action names
    and `is_mutating`.
  - Add the events `RelayChanged`, `HostReachChanged` and `HostQueryAnswered` in
    `proto/event.rs`.
  - Add round-trip tests for every new variant, and regenerate `web/src/protocol/generated`.
  - Done in `crates/keel-protocol/src/relay.rs`, tested in `tests/roundtrip_067.rs`. Added
    `PreviewRelayRelease` (wrapping T059's `release_relay`) and `RelayLinkKeyState`. `HostReach`
    is tagged `kind`: `connected{path}`, `disconnected`, `relay_unreachable{relays}`,
    `relay_revoked{relay}` and `no_path`.
- [X] T029 [P] [US3] Test first, unit tests for `derive_reach(host, zone, relays_live)` in
  `server/src/relays.rs`, covering every row of data-model.md's "Host (display, derived)" table:
  - connected direct or via a relay;
  - disconnected;
  - relay unreachable;
  - relay revoked;
  - no path.
- [X] T030 [US3] Implement the zone and relay handlers in `server/src/relays.rs` and route them
  in `server/src/api.rs`:
  - `ListZones`, `PreviewZoneCreate`, `PreviewZoneUpdate` and `PreviewZoneRemove`. The remove
    preview lists blocking hosts and relays.
  - `PreviewHostZone`, where the zone must be in the host's workspace.
  - `ListRelays`, `PreviewRelayRegister` and `PreviewRelayUpdate` (both step-up, wrapping T023;
    the preview shows the fingerprint and Keel's relay-link key line), and `PreviewRelayRevoke`
    (the preview lists connected hosts, wrapping T027).
  - `GetRelayLinkKeys`, and `PreviewRelayLinkKeyRotate`, `PreviewRelayLinkKeyActivate` and
    `PreviewRelayLinkKeyRetire` (instance admin, step-up, mirroring `PreviewSshCaRotate`).

  Every confirm audits in the same transaction. Add tests in `server/tests/relays.rs`.

  Deviations:
  - The handlers live in `server/src/zones.rs`, routed in `dispatch.rs`. `relays.rs` gained
    `update_relay` and the link-key rotate, activate and retire functions.
  - `PreviewRelayRelease` (step-up) shows the quarantine reason, when it happened, and the
    `relay.refused` attempts since.
  - `PreviewRelayRevoke` asks for no step-up, so a compromised relay can be cut off fast. Zone
    changes ask for none either.
  - Migration `0020_zone_remove.sql` grants `delete` on zones (0019 is taken by another branch).
    The foreign keys have no cascade, so a zone that ever had a relay, even a revoked one, can't
    be removed. The remove preview lists such relays as `relay <name> (revoked)`.
- [X] T031 [US3] Fill `HostInfo.zone` and `reach` in `ListHosts`. Push `RelayChanged` on state
  changes (RTT and stream-count updates at most every 30 s), and `HostReachChanged` when a host's
  derived reach changes (`server/src/relays.rs`, `server/src/ws.rs`). In
  `server/tests/relays.rs`, stopping the relay must emit `HostReachChanged` to
  `relay_unreachable` within one health interval.
  - The pushes come from a publisher task that `RelaySupervisor` owns, through the existing
    workspace broadcast rather than `ws.rs`. `queue.rs` also pushes reach on runner connect and
    disconnect, and `PreviewHostZone` pushes it when a host changes zone. The test uses a relay
    nothing listens on, so it never connects.
- [X] T032 [US3] Add a run preview warning in `server/src/preview.rs`. It fires when targets
  include `relays.host_id` of the only non-revoked relay of a zone, and other targets are in
  that zone. It names the relay host and the dependent hosts. Test it in
  `server/tests/preview.rs`.
  - The warning comes from `relays::run_warnings`, which the run-start preview in `runs.rs`
    adds next to the spec 066 warnings. It names at most 10 hosts, then "and N more". The tests
    are `relays::run_warnings` in `server/tests/relays.rs` and the wording in unit tests. No
    test drives the full run preview.
- [X] T033 [P] [US3] Add the fake backend `web/tests/support/fake-relays.ts`, which serves zones,
  relays, link keys, reach and events for the commands above.
  - `FakeRelays(backend, hosts, workspace?)` scripts zones, relays and placements, and
    `setState` pushes `RelayChanged`, `HostState` and `HostReachChanged` together. Preview texts
    mirror the server's. It also answers `PreviewRunStart` with the relay-host warning.
- [X] T034 [US3] Update `web/src/features/hosts/`:
  - `reach.ts`, with grouping helpers and reach text, plus Vitest tests;
  - `HostsPage.tsx`, grouped by zone with `direct` first, relay chips (an icon and colour for
    state; RTT and streams at ≥ 768 px), and a Path column below state and above versions;
  - an inspector "Change zone" action through preview/confirm.
  - Done in `reach.ts` (tested in `tests/unit/reach.test.ts`), `ZoneParts.tsx` and
    `HostsPage.tsx`. Grouping turns on only once the workspace has a zone or a zoned host, so a
    workspace without relays keeps the single "Hosts" table. The inspector shows zone and current
    path; the last path and its advisory peer are not on the wire, so they are not shown.
- [X] T035 [US3] Add `web/src/features/settings/RelaysTab.tsx` and `relays-store.ts`, registered
  in `tabs.ts` and lazy-loaded. It shows Keel's relay-link key with a copy button, labelled
  "Add this to `keel_relay_hub_keys`", the zones list, and the relays list with
  Register/Edit/Revoke dialogs per contracts/ui.md.
  - The tab is offered to instance admins and workspace admins. A workspace admin manages zones
    and relays but does not see the link key (the Register preview carries its line). A
    quarantined relay shows its reason and Release. Its "Learn more" links point at the hosts
    guide until the relays guide lands (T049).
- [X] T036 [US3] Playwright `web/tests/e2e/s067/hosts-relays.spec.ts` and `relays-tab.spec.ts`:
  - grouping and paths;
  - a relay going unreachable flips only its zone;
  - register with a fingerprint preview;
  - revoke naming the connected hosts;
  - link key copy;
  - the run preview warning.

  Run with `--project=s067`.
  - 11 tests in `hosts-relays.spec.ts` and `relays-tab.spec.ts` on a shared `scene.ts`. They also
    cover phone-width chips, Change zone, quarantine release, zones and the workspace-admin view.
    The tab lists in `settings-system.spec.ts` and `s066/settings-workspace.spec.ts` now include
    Relays; those specs were not run here.

## Phase 6: User Story 4, more than one relay per zone (P2)

**Goal**: if one relay in a zone goes away, runners reconnect through the other and runs keep
dispatching.

**Independent test**: `cargo test -p keel-server --test relay_failover`.

- [ ] T037 [P] [US4] `server/tests/relay_failover.rs` uses two in-process relays in one zone and
  a runner with both in `--dial`:
  - stop the relay in use, and the runner reconnects through the other within 60 s with the
    path updated;
  - a run in progress on the stopped relay is handled as a lost connection and never runs twice;
  - a run queued meanwhile dispatches after the reconnect.
- [ ] T038 [US4] Add a test in `runner/tests/dial.rs` for a dial list with relays first and the
  direct address last. When the relays are down and the direct address answers, the runner
  connects direct, and Keel shows `direct`. Fix whatever T037/T038 reveal in
  `runner/src/runner/mod.rs` or `server/src/relays.rs`.

## Phase 7: User Story 5, typed questions to hosts (P3)

**Goal**: owners ask hosts typed questions over the runner session, direct or relayed. There is
no shell and no tunnel.

**Independent test**: ask a host behind a relay for an allowed unit's state and its disk health,
and get answers. Ask for a unit outside `queries.toml`, and get a refusal audited on both sides.

- [ ] T039 [P] [US5] Add `Query { query_id, kind, params, asked_by }`, `QueryAnswer { query_id,
  answer }` and `QueryRefused { query_id, reason }` to `exec/frames.rs`.
  - `reason` is one of `not_allowed`, `bad_params`, `unavailable`, `too_large` or `busy`.
  - Wire them into `Frame::name`, `subsystem` and `may_be_sent_by`.
  - Add round-trip tests.
- [ ] T040 [P] [US5] Test first, `runner/tests/queries.rs`:
  - parsing `queries.toml` with `deny_unknown_fields`;
  - a missing file refuses every query with `not_allowed`;
  - unit matching is exact or a trailing `*` on a literal prefix, with the unit matching
    `^[A-Za-z0-9@._:-]{1,256}$`;
  - path matching is exact or a path below an allowed one, absolute, normalised, with no `..`,
    and at most 4096 bytes;
  - answers over 64 KiB are refused `too_large`, and a fifth query in flight is refused `busy`;
  - an assertion that the `systemctl` invocation is an argv array with `--` before the unit.
- [ ] T041 [US5] Implement `runner/src/queries.rs`, which loads
  `/etc/keel-runner/trust/queries.toml` and answers each kind per contracts/relay.md:
  - `facts` and `applied_revision` from data the runner already has;
  - `unit_state` with `systemctl show --property=LoadState,ActiveState,SubState,
    ActiveEnterTimestamp -- <unit>` through `Command` argv;
  - `path_free` with `statvfs`;
  - `disk_health` from `/proc/mounts` and `/sys/block`, plus `smartctl --json` argv when
    installed, routed to the executor over its socket like `ReadLedger`.

  Append each query to `/var/lib/keel-runner/queries.log` as JSON lines, rotated at 1 MiB
  keeping 3. Advertise `capabilities: ["query-v1"]` in `Hello`. Dispatch in
  `runner/src/runner/session.rs` without blocking run frames.
- [ ] T042 [US5] Implement `server/src/host_queries.rs`:
  - `AskHost` refuses early when the host is offline or lacks `query-v1` (`no_capability`), and
    otherwise inserts into `host_queries` and sends `Query` through the run hub (`queue.rs`);
  - a 15 s timeout gives `timeout`, and the answer, refusal or timeout updates the row and pushes
    `HostQueryAnswered`;
  - `ListHostQueries` lists them;
  - every query is audited as `host.query` with kind, params and outcome, never the answer;
  - the newest 200 rows per host are kept, with older ones deleted in the same transaction as
    the insert.

  Test it in `server/tests/host_queries.rs`, both direct and through a relay.
- [ ] T043 [US5] Add `web/src/features/hosts/QueriesPanel.tsx` in the host inspector, with kind
  picker, parameter field, Ask, and answers newest first with time, asker and outcome. A host
  without `query-v1` shows "runner too old for queries". Add Vitest, and Playwright
  `web/tests/e2e/s067/queries.spec.ts` for answered, refused and too old.

## Phase 8: Release-file cache (FR-010, MAY), blocked on plan D1

**Do not start until T057 (D1) has merged.**

- [ ] T044 [US1] Server side, in `server/src/relays.rs`, answer `CacheGet` from the files behind
  the `/runner` route only. Paths must be under `runner/<release>/` and named in the release
  manifest. Send 512 KiB `CacheChunk`s, allow at most 4 concurrent requests and 64 MiB per
  release per relay, and answer `CacheMiss` (`unavailable`, `not_found` or `too_large`).
  Before D1 lands, implement only the `CacheMiss { reason: "unavailable" }` answer and its test.
- [ ] T045 [US1] Relay side, in `relay/src/cache.rs`, serve `GET /runner/<release>/<path>` on
  `http_listen` from `/var/lib/keel-relay/cache/<release>/`, fetching via `CacheGet` on a miss.
  Add a test that a tampered cache file still fails the host's checksum check.

## Phase 9: Polish, packaging and cross-cutting

- [ ] T046 [P] Add the OTel spans `relay.link` (relay id, state), `relay.stream` (relay id and
  byte counts, never contents) and `host.query` (kind, outcome) in `server/src/relays.rs`,
  `server/src/host_queries.rs` and `relay/src/forward.rs`.
- [ ] T047 [P] Add `apps/keel/compose.relay.yaml` per quickstart.md §2:
  - networks `hub` and `spoke`, with `relay` on both and `runner` on `spoke` only;
  - `restart: ${RESTART_POLICY:-no}` on every service.

  Walk through the quickstart §2 steps by hand and record the results in this file under T047.
- [ ] T048 Add `keel-relay` to the release build that produces `keel-runner`: the same checksums
  file, x86_64 and aarch64 (`apps/keel/Makefile`, release scripts).
- [ ] T049 [P] Write `docs/guides/keel/relays.md`, covering:
  - when to use a relay;
  - zones;
  - registering by pasting keys both ways;
  - the trust properties (the relay can only drop or delay);
  - relays as host services, never cluster workloads;
  - queries and `queries.toml`.

  Also add an architecture note to `CONTRIBUTING.md`.
- [X] T050 **(../ansible 48cb22d, fluxified session)** Add `keel_runner_dial`,
  `keel_runner_download_origin` and `keel_runner_queries` to `roles/keel_runner`, with the
  validation in contracts/ansible.md. The unit gets one `--dial` per entry.
- [X] T051 **(../ansible aadf5d5, fluxified session)** Add a new `roles/keel_relay` and a `keel-relay`
  profile per contracts/ansible.md, including the checksum fail-closed install and printing the
  relay key.
- [ ] T052 Main session only: run `scripts/hygiene.sh`, then the full gate once on the combined
  branch (`scripts/ci.sh`). Report wall time, the slowest steps and the delta from the last pass.
- [ ] T053 Brochure screens last: `polish-screens` shots of the Hosts page with zones and of
  Settings → Relays at 360 and 1280 px, then the impeccable pass over both.
- [ ] T054 After D1–D3 land: SC-001 on real hosts (quickstart.md §3). Install the relay on one
  OVH host, set `keel_runner_dial` on `ns534168` and `ns5004891`, confirm no inbound port is
  open at home and no tailnet is in use, and run a signed job on each.

## Dependencies & execution order

- Phase 1 → Phase 2 → stories.
- US1 (Phase 3) is the MVP and blocks US2, US4 and US5. US2 ships with US1 and is not optional.
- US3 needs T023 and T027, and can start after Phase 2 in parallel with the US1 implementation,
  apart from T030/T031, which need the supervisor (T020).
- US4 needs US1. US5 needs Phase 2 only for the frames, but its relayed test needs US1.
- Phase 8 waits for T057 (D1). T050/T051 are with the fluxified session. T054 is blocked on D1–D3.

## Parallel opportunities

- Phase 2: T005, T007, T009 and T013 touch separate files.
- US1: T014, T015 and T016 (tests) in parallel. Then the relay half (T017–T019), the server half
  (T020, T021, T023) and the runner half (T022) can go to three agents in separate worktrees.
- US2: T024 and T025 in parallel.
- US3: T028, T029 and T033 in parallel. Then server (T030–T032) and web (T034–T036).
- US5: T039 and T040 in parallel. Then runner (T041) and server (T042), then web (T043).

## Implementation strategy

1. **MVP = Phases 1–4** (US1 and US2 together): a relayed host runs signed jobs, and a hostile
   relay is proven harmless. US1 must not ship without US2 (spec).
2. Then US3, so operators can see relays.
3. Then US4, then US5.
4. Phase 8 when D1 lands. The ../ansible tasks when the owner says go.
5. Brochure screens last (T053).
