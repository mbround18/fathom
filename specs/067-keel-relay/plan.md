# Implementation Plan: Keel relays for hosts Keel cannot be reached from

**Branch**: `067-keel-relay` | **Date**: 2026-11-17 | **Spec**: [spec.md](spec.md)

## Summary

A new `keel-relay` binary sits in a spoke network. Keel dials it over SSH (russh, the same
algorithm and key handling as the runner listener). Runners in the spoke dial the relay's local
port, and the relay hands each TCP connection to Keel as a `forwarded-tcpip` channel. Keel runs
its existing runner SSH server over that channel, so the runner contract, host-certificate
checks, signed mode and approvals are unchanged end to end, and the relay sees only ciphertext.
Keys are pinned both ways by pasting (R2). On top of that this plan adds:

- zones and per-session paths on the Hosts page, and a Relays tab in Settings;
- a runner dial list separate from the verified server name;
- typed host queries over the existing runner session (Story 5);
- an optional relay-side cache of runner release files.

Research: [research.md](research.md). Both open questions in the spec are resolved there (R1,
R2), as the owner confirmed on 2026-11-17.

## Technical Context

**Language/Version**: Rust (edition 2024, `rust-toolchain.toml`) for keel-exec, keel-protocol,
keel-server, keel-runner and the new keel-relay. TypeScript, React and Vite for `apps/keel/web`.
Markdown for `docs/guides/keel`.

**Primary Dependencies**: existing ones only.

- `russh` 0.65 (server and runner already use it; the relay gets the same version), `tokio`,
  `serde`, `toml`, `tracing`, `clap`.
- keel-exec's `server_config`, `client_config`, `codec`, `ca`.
- Release cache HTTP: `axum` (already the server's) or a minimal `hyper` service. Pick whichever
  keeps the relay binary smaller (measure; the relay must stay small).
- No D-Bus crate (R8). No new runtime crates. One new dev-dependency, `proptest`, for the
  property tests in tasks T064 (owner decision, 2026-11-17).

**Storage**: PostgreSQL migration `0017_relays.sql` ([data-model.md](data-model.md)). The relay
keeps only its host key and release cache on disk.

**Testing**:

- `cargo test -p keel-exec relay` for link policy and frame round trips.
- `cargo test -p keel-relay`.
- `cargo test -p keel-runner`: dial list, queries, hostile relay.
- `cargo test -p keel-server` integration tests `relay_link`, `hostile_relay`, `relays`,
  `relay_failover` and `host_queries`, all Postgres-backed. Each must fail loudly when
  `KEEL_TEST_DATABASE_URL` is unset (see Dependencies D4).
- Vitest for the reach derivation and grouping helpers.
- Playwright project `s067`, plus `polish-screens`.
- The two-network compose scenario in [quickstart.md](quickstart.md) §2.

**Target Platform**: the Keel server container (amd64, arm64). `keel-relay` and `keel-runner` on
Debian with systemd, x86_64 and aarch64.

**Project Type**: web service + host agents (server, runner, relay) + web UI.

**Performance Goals**:

- One relay hop adds at most the link's RTT to frame delivery (SC-004).
- Log streaming keeps up with 1 MiB of output per minute through a relay. That is trivial for
  SSH channel windows, but test it with a fixture job that writes 2 MiB in 60 s.
- Health interval 30 s; runner reconnect ≤ 60 s (SC-003).

**Constraints**:

- The relay never terminates a runner session (FR-002).
- The link carries only runner streams, health and `CacheGet` (FR-004).
- Nothing on a host builds a shell command from query parameters (FR-013).
- An old runner (no `--dial`, no `query-v1`) keeps working direct.

**Scale/Scope**: a handful of zones and relays per workspace; ≤ 256 runner streams per relay.

## Constitution Check

*Pre-research and post-design: both pass. No entries in Complexity Tracking.*

| Principle | How this plan complies |
|---|---|
| I. Explicit mutation | Every zone, relay, host-zone and link-key change uses Preview → Confirm. Revoke names the relay and the connected hosts. Live state is shown only as the supervisor reports it. |
| II. One contract | Every UI change goes through `keel-protocol` (contracts/protocol.md), with additive types and ts-rs bindings. Link frames are defined once, in keel-exec, and shared by server and relay. |
| III. Tested at boundaries | Every new command and event, and every control frame, gets a round-trip test. Each story has a test that fails without it (quickstart §1), plus Playwright `s067` against the fake backend. The hostile-relay suite is SC-002. |
| IV. Density | The Hosts grouping and Path column follow existing column priorities. Relay chips gain RTT and stream count at wider widths. Everything works at 360 px and by keyboard. |
| V. Fast and quiet | Relay RTT and stream-count events are throttled to 30 s. The supervisor is idle between pings. |
| VI. User data | Audit entries are written in the same transaction as each change. Keel's relay-link private key is sealed like the SSH CAs. Relays and keys are revoked, never deleted. No secret crosses the relay in clear: enrollment tokens and secrets stay inside the runner SSH session. |
| Keel server rules | The server still executes nothing and holds no key that reaches a managed host's shell. The relay-link key opens only the relay's embedded SSH server, which offers one subsystem and no shell, `exec` or forwarding. |
| Keel host runner rules | Runners still connect outbound only. Signed mode, the ledger and approvals are untouched, and host-side verification of the server is unchanged (R3, R4). |
| Stack | Rust, tokio, russh, Axum, sqlx, React and Vite: nothing replaced or added. |
| Global rules (agnostic core + adapters, OTel, feature flag) | Link policy and frames sit in keel-exec, with no I/O policy in the server app. OTel spans: `relay.link` (attributes: relay id, state), `relay.stream` (relay id, bytes; never contents), `host.query` (kind, outcome). The runtime flag `KEEL_RELAYS` (default on once US1–US3 land) gates the supervisor and the Settings tab, so a server can be run with relays off. |

## Dependencies (outside this spec; they gate real-host testing)

Found by the 2026-11-17 readiness audit. SC-001 cannot be checked on `ns534168`/`ns5004891`
until D1–D3 land. On 2026-11-17 the owner brought all four into this branch (tasks.md Phase 0):
D2 is one run per host, and D4 makes CI start the test database and run every keel-server test.

| # | Gap | Where | Owner spec |
|---|---|---|---|
| D1 | Keel has no `/runner/<release>/…` download route, and the image ships no runner files. The Ansible role expects one (`../ansible/roles/keel_runner/tasks/binaries.yml:29`, `keel_runner_conn.origin/runner/<release>/`). This also blocks the relay release cache (R7, `CacheGet` answers `unavailable` until then). | server routes, `apps/keel/Dockerfile` | 055 T128, T147, T149 |
| D2 | `dispatch_ready` has no per-host run lock: two runs can be claimed for one host at once. Relayed sessions that drop and reconnect make this likelier. | `apps/keel/server/src/queue.rs:1556` | 055 |
| D3 | Join tokens are unimplemented (`keel-join-v1`, `keel_join_token`). Enrollment through a relay works with single-use enroll tokens, but fleet onboarding of both OVH hosts expects join tokens. | runner listener, runner, role | 055 T137–T149 |
| D4 | 19 of the 27 keel-server test files skip silently without `KEEL_TEST_DATABASE_URL`, so a green `cargo test` can mean "nothing ran". This plan's new server tests must refuse to skip quietly: under `CI=1` they fail when the variable is unset, and otherwise they print one loud skip line. The fix for the existing 19 files belongs to 055/058. | `apps/keel/server/tests/common` | 055 / 058 |

Also: the fluxified Claude session writes the `keel_relay` role and the `keel_runner` variables
in `../ansible` (R11, contracts/ansible.md), as the owner asked.

## Phasing (story order for tasks.md)

1. **Foundation**: migration `0017`, link policy and frames in keel-exec, a `RunnerPath`
   threaded through `runner_listener` and `queue::attach`, Keel's relay-link key, the
   `keel-relay` crate skeleton and config.
2. **US1** (P1): the relay binary (link server, runner listener, forwarding), Keel's
   `RelaySupervisor` (dial, control, accepting `forwarded-tcpip`, running `KeelSshHandler` over
   the channel), the runner `--dial` list, and the `relay_link` test. **MVP.**
3. **US2** (P1): the hostile-relay suites, revocation closing link and streams, and an audit
   entry for every refusal. Ships with US1.
4. **US3** (P1): zones, protocol commands and events, `HostReach`, the Hosts page grouping and
   Path column, the Settings → Relays tab, and the run-preview warning.
5. **US4** (P2): a failover test with two relays, plus the dial-list edge cases (direct address
   listed after relays).
6. **US5** (P3): query frames, `queries.toml`, the runner answer code, `host_queries`, and the
   inspector panel.
7. **Release cache** (FR-010, MAY): `CacheGet` and relay HTTP. Blocked on D1.
8. **Packaging and docs**: the `keel-relay` systemd unit and release artifacts, the
   `compose.relay.yaml` scenario, `docs/guides/keel/relays.md`, a CONTRIBUTING note, the
   `../ansible` role (with the owner's go-ahead), and brochure screens last.

## Project Structure

### Documentation (this feature)

```text
specs/067-keel-relay/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── relay.md       # link SSH policy, control frames, runner additions, hostile matrix
│   ├── protocol.md    # keel-protocol additions
│   ├── ui.md
│   └── ansible.md     # what ../ansible must do
├── checklists/
└── tasks.md           # /speckit-tasks
```

### Source Code (repository root)

```text
crates/keel-exec/src/relay.rs                 (new: control frames, link client/server policy)
crates/keel-exec/src/frames.rs                (+ Query/QueryAnswer/QueryRefused, Hello.capabilities)
crates/keel-protocol/src/relay.rs             (new: zones, relays, reach, queries)
crates/keel-protocol/src/{cmd,event,host,lib}.rs
apps/keel/relay/                              (new crate: lib + bin keel-relay)
  Cargo.toml, src/{lib,config,link,forward,cache,key}.rs, src/bin/keel-relay.rs,
  tests/{link,limits}.rs, systemd/keel-relay.service
apps/keel/server/migrations/0017_relays.sql
apps/keel/server/src/relays.rs                (new: supervisor, link client, registration, zones)
apps/keel/server/src/host_queries.rs          (new)
apps/keel/server/src/runner_listener.rs       (RunnerPath; serve over any stream)
apps/keel/server/src/queue.rs                 (path in session state; Query routing)
apps/keel/server/src/{config,api,serve,settings,preview}.rs
apps/keel/server/tests/{relay_link,hostile_relay,relays,relay_failover,host_queries}.rs
apps/keel/runner/src/runner/{mod,session}.rs  (--dial, capabilities, query dispatch)
apps/keel/runner/src/queries.rs               (new: queries.toml, answerers)
apps/keel/runner/tests/{hostile_relay,dial,queries}.rs
apps/keel/web/src/features/hosts/{HostsPage.tsx,store.ts,reach.ts,QueriesPanel.tsx}
apps/keel/web/src/features/settings/{RelaysTab.tsx,relays-store.ts,tabs.ts}
apps/keel/web/tests/e2e/s067/, tests/support/fake-relays.ts
apps/keel/compose.relay.yaml
docs/guides/keel/relays.md
```

**Structure decision**: the relay is an execution layer like the runner (`apps/keel/relay`). The
policy it shares with the server lives in `crates/keel-exec`, so neither binary owns the
contract. Add `apps/keel/relay` to the workspace `members` and to the release build that already
produces `keel-runner`.

## Complexity Tracking

None.
