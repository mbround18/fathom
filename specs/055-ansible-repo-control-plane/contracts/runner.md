# Contract: server ↔ runner (`crates/keel-exec`)

## Connection

- The runner connects to the server's **runner listener** (`--runner-listen`, internal network
  only), using `GET /runner/v1/ws`.
- **Enrollment:** the runner generates an Ed25519 key on first start, stored on its own volume. It
  sends the key once together with the one-time enrollment token from `PreviewRunnerEnroll`.
  The server records the key's fingerprint against the pool.
- **Every connection:** the server sends a nonce, and the runner signs it. The runner checks the
  server's instance key against the fingerprint pinned when it enrolled. Revoked runners are
  refused.
- Frames are JSON, and the message types are defined in Rust in `keel-exec`. They do not appear
  in the web protocol.

## Frames

| Direction | Frame | Notes |
|---|---|---|
| R → S | `Hello { runner_id, version, ansible_core, python, state }` | The server refuses `ansible_core` versions below its minimum (R3). |
| S → R | `Assign { run_id, bundle_size, deadline }` | Sent only to an `idle` runner of a pool that serves the run's workspace. |
| S → R | `Bundle { run_id, chunk, last }` | A tar holding `repo/` (the tree at the pinned commit), `inventory/` (rendered from the pinned revision, with `group_vars` and `host_vars`), `job.json` (playbook, limit, extra vars, timeout) and `secrets/` (credential files and `env.json`). Its SHA-256 is in the final chunk. |
| R → S | `Started { run_id, pid_namespace_ok }` | |
| R → S | `Event { run_id, seq, json }` | One `keel_events` line (R3). |
| R → S | `Log { run_id, seq, stream, bytes }` | Batched every 100 ms or 64 KiB, whichever comes first. |
| S → R | `Cancel { run_id }` | Sends SIGINT, then after 30 s SIGKILL to the run's session. |
| R → S | `Finished { run_id, exit_code, stats }` | |
| R → S | `Cleaned { run_id, leftover_processes: u32 }` | If `leftover_processes` is above zero, the runner reports `Tainted` and is never assigned again until it is restarted (R2). |
| both | `Ping` / `Pong` | Every 15 s. After 3 missed pings a running run becomes `failed (runner_lost)`. |

## Guarantees

- Secrets travel only in `Bundle`. They are never logged by either side, and they live in the
  runner's `tmpfs` only between `Assign` and `Cleaned`.
- The server writes `run_events` and `run_log_chunks` in batches, and forwards them live to
  subscribed clients. `seq` makes a reconnecting client's `SubscribeRun { from_seq }` resume
  without gaps.
- The runner holds no database credentials, no master key and no receiver secrets.
