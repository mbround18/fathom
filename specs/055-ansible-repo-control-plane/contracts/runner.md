# Contract: server ↔ runner (`crates/keel-exec`)

## Connection (research R17)

- The runner dials the server's **runner listener** (`--runner-listen`) at
  `wss://<host>:7831/runner/v1/ws`.
  - TLS 1.3 only, through rustls. There is no plaintext mode, and a `ws://` URL is refused at
    start.
- **Server authentication:** the runner accepts only a server certificate whose SHA-256
  fingerprint it has pinned. It ignores CA chains and hostnames.
- **Runner authentication:** the runner always presents its self-signed Ed25519 client
  certificate, kept in `--state-dir`.
- **Enrollment:**
  1. `PreviewRunnerEnroll` yields `keel-enroll-v1.<secret>.<server-fingerprint>`.
  2. The runner pins the fingerprint from the token, connects, and sends `Enroll { token }` as its
     only frame.
  3. The server checks the token (single-use, 15-minute TTL, stored hashed), records the client
     certificate's fingerprint against the pool, and replies `Enrolled { runner_id }`.
  - A wrong or used token closes the connection and is audited.
- **Every later connection:** the client certificate must belong to a runner that is not revoked,
  or the TLS handshake is followed by a close with `unknown_runner`. Revoking a runner closes its
  live connection.
- **Server certificate rotation:** `RotateServerCert { fingerprint, old_expires_at }` arrives
  over the authenticated channel. The runner pins both fingerprints until `old_expires_at`.
- Frames are JSON, and the message types are defined in Rust in `keel-exec`. They do not appear
  in the web protocol.

## Frames

| Direction | Frame | Notes |
|---|---|---|
| R → S | `Enroll { token }` | First connection only; see above. |
| S → R | `Enrolled { runner_id }` | |
| R → S | `Hello { runner_id, version, ansible_core, python, state }` | The server refuses `ansible_core` versions below its minimum (R3). |
| S → R | `RotateServerCert { fingerprint, old_expires_at }` | |
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

- Secrets travel only in `Bundle`, inside the mutually authenticated TLS channel. They are never logged by either side, and they live in the
  runner's `tmpfs` only between `Assign` and `Cleaned`.
- The server writes `run_events` and `run_log_chunks` in batches, and forwards them live to
  subscribed clients. `seq` makes a reconnecting client's `SubscribeRun { from_seq }` resume
  without gaps.
- The runner holds no database credentials, no master key and no receiver secrets.
