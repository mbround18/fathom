# Quickstart: validating Keel relays (spec 067)

Contracts: [relay.md](contracts/relay.md), [protocol.md](contracts/protocol.md),
[ui.md](contracts/ui.md), [ansible.md](contracts/ansible.md). Data: [data-model.md](data-model.md).

## Prerequisites

- A Postgres for keel-server tests: `KEEL_TEST_DATABASE_URL` **must** be set. Without it 19 of
  the 27 keel-server test files skip silently, including every test below marked (pg). Check the
  test output says `running N tests` with N > 0 for each file.
- `CARGO_BUILD_JOBS=4` when other agents are active. Subagents never run `scripts/ci.sh`.

## 1. Automated, in-process (the gate for each story)

| Story | Command | Proves |
|---|---|---|
| link policy | `cargo test -p keel-exec relay` | control-frame round trips; handler refuses every channel/request in the contract table |
| relay | `cargo test -p keel-relay` | config parsing fails closed; no link → runner TCP closed at once; `max_streams`; key generation and `key` output |
| US1 (pg) | `cargo test -p keel-server --test relay_link` | Keel with **no runner listener bound** dials an in-process relay; a real `keel-runner` dialling only the relay enrolls, connects (`path = relay`), runs a fixture job and streams its log; stopping the relay → host `relay unreachable` within one interval |
| US2 | `cargo test -p keel-runner --test hostile_relay` and `cargo test -p keel-server --test hostile_relay` (pg) | the SC-002 matrix in contracts/relay.md; no run directory, ledger unchanged, an audit row per attempt |
| US3 (pg) | `cargo test -p keel-server --test relays` | zone/relay CRUD through preview/confirm with audit; revocation closes link and streams; `HostReach` derivation table |
| US4 (pg) | `cargo test -p keel-server --test relay_failover` | runner with two `--dial` relays moves to the second when the first stops; a run in progress is lost, not run twice; a queued run dispatches after reconnect |
| US5 | `cargo test -p keel-runner queries` then `cargo test -p keel-server --test host_queries` (pg) | allow-list matching, argv-only command construction, refusals, 64 KiB cap, `busy`; audit on both sides |
| runner dial | `cargo test -p keel-runner dial` | dial order, a dialled address answering with a wrong host cert is skipped, verified name is always `--server` |
| UI | `pnpm --filter @companion/keel-web test` then `pnpm --filter @companion/keel-web exec playwright test --project s067` | contracts/ui.md E2E list |

## 2. Two-network compose scenario (manual, before real hosts)

`apps/keel/compose.relay.yaml` (added by this feature, `restart: ${RESTART_POLICY:-no}`) puts:

- `keel` and `postgres` on network `hub`,
- `relay` on `hub` and `spoke`,
- `runner` (a Debian container running `keel-runner` + `keel-executor` with a fixture trust dir)
  on `spoke` only.

```bash
docker compose -f apps/keel/compose.yaml -f apps/keel/compose.relay.yaml up -d
docker compose … exec relay keel-relay key          # paste into Settings → Relays
# In Keel: Settings → Relays → copy the relay-link key into the relay's hub_keys, restart relay
# Register relay "spoke-1" in zone "spoke", dial address relay:7832
# Enroll the runner with --server keel.test:7831 --dial relay:7831
```

Expected:

1. Settings → Relays shows `spoke-1` connected with an RTT. Hosts shows the runner under zone
   `spoke`, `connected · via spoke-1`.
2. `docker compose … exec runner getent hosts keel` fails (no route to the hub): the runner only
   reaches Keel through the relay.
3. Push a signed fixture commit; the run completes and its log streams.
4. `docker compose … stop relay`: within 30 s the relay is `unreachable` and the host shows
   `disconnected (relay unreachable)`. Start it again; the host reconnects within 60 s.
5. Ask the host `unit_state` for an allowed unit, then for one not in `queries.toml`: answered,
   then refused, both in the audit log and in the runner's `queries.log`.
6. Revoke the relay: link and session close at once, host shows `disconnected (relay revoked)`.

Tear down with `docker compose … down` (no `-v` unless the volumes are this scenario's own).

## 3. Real hosts (after the dependencies in plan.md land)

`ns534168` then `ns5004891` (SC-001): `keel-relay` profile on one OVH host, `keel_runner_dial`
on both, port 7832 open to the home egress address only. Confirm no inbound port is open at home
and the hosts are not using the tailnet (`tailscale status` down or absent).
