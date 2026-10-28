# Contract: server ↔ host runner (`crates/keel-exec`)

The runner is installed on each managed host by the base profile (contracts/host-install.md).
It has two halves (research R2): `keel-runner` (network side, user `keel-runner`) and
`keel-executor` (runs Ansible, user `keel-exec`). This contract covers the SSH channel between
Keel and `keel-runner`, and the local socket between the two halves.

## SSH connection (research R17)

- The runner connects to `keel_runner_server` (default port 7831) as an SSH client. The user name
  is `runner`.
- **Server authentication:** the server must present an SSH host certificate signed by the host
  CA in `/etc/keel-runner/trust/host_ca.pub`. A bare host key, a certificate from another CA, or
  an expired certificate ends the connection before anything is sent.
- **Algorithms:** `ssh-ed25519` keys and certificates only; key exchange `curve25519-sha256`;
  ciphers `chacha20-poly1305@openssh.com` or `aes256-gcm@openssh.com`. Anything else is refused
  by both sides.
- **Runner authentication:**
  - Normally: the runner certificate (`/var/lib/keel-runner/id_ed25519-cert.pub`). The server
    checks the runner CA signature, validity, the serial against the KRL, and that the key is the
    enrolled key of a host that is not revoked.
  - Expired certificate: the bare enrolled key, accepted only within `--runner-max-offline` of the
    certificate's expiry and only for the `RenewCert` frame.
  - Unknown key: only the `keel-enroll-v1` and `keel-join-v1` subsystems.
- **Refused by the server:** shell, exec, pty, env, port forwarding (both directions), agent and
  X11 forwarding. The only subsystems are `keel-enroll-v1`, `keel-join-v1` (contracts/join.md) and
  `keel-runner-v1`.
- Frames are length-prefixed (u32, big-endian) JSON, at most 16 MiB each, defined in Rust in
  `keel-exec`. They do not appear in the web protocol.

## Enrollment (`keel-enroll-v1`)

1. `PreviewHostEnroll { inventory_name, profiles }` yields
   `keel-enroll-v1.<secret>.<host CA SHA-256 fingerprint>`.
2. The base profile writes the token to `/var/lib/keel-runner/enroll-token` (`0400`, owner
   `keel-runner`) and starts the service. The runner checks that the token's fingerprint matches
   the pinned `host_ca.pub`; if it does not, it refuses to connect.
3. Over `keel-enroll-v1` it sends `Enroll { token, inventory_name, allowed_profiles,
   require_signed }`.
4. The server checks the token (single-use, 15-minute TTL, stored hashed), that `inventory_name`
   matches the token, and that the token's profiles are within `allowed_profiles`. It records the
   key and replies `Enrolled { host_id, cert }`. The runner deletes the token file.
- Any failure closes the connection, burns the token, and is audited.

## Frames (`keel-runner-v1`)

| Direction | Frame | Notes |
|---|---|---|
| R → S | `Hello { host_id, version, ansible_core, python, allowed_profiles, require_signed, override_quorum, ledger_head, bases: [{ job, commit }] }` | `bases` come from the executor's ledger. The server refuses `ansible_core` below its minimum (research R3). |
| S → R | `RenewCert { }` / R → S `RenewCert { }` | Either side may start it. The server replies `Cert { cert }`, with principals `host:<name>` and `profile:<p>` for assigned ∩ allowed profiles. |
| S → R | `Krl { bytes }` | Sent after `Hello` and on change; the runner keeps it for its own checks of other Keel-signed material. |
| S → R | `Assign { run_id, workspace, binding, job, base, target, deadline, pack_size, quorum }` | Sent only for jobs whose profiles the host has. `base` is Keel's belief; the executor checks it against its ledger. `workspace` and `binding` are lowercase hyphenated UUIDs; with `run_id`, `job` (the job name), `base` and `target` they rebuild the override challenge (signed-mode.md). `quorum` (optional integer) is Keel's override quorum for the run; the host requires `max(policy.override_quorum, quorum)`. |
| S → R | `Pack { run_id, chunk, last }` | A Git pack of `base..target` (or the full history up to `target` for a baseline). Its SHA-256 is in the last chunk. |
| S → R | `Secrets { run_id, items: [{ name, kind, value }] }` | Only credentials the job references and that are granted to one of the host's profiles. |
| S → R | `Approvals { run_id, items: [{ kind, approver, credential_id, authenticator_data, client_data_json, signature, expires_at, comment, hosts }] }` | Override and baseline approvals (research R20). `expires_at` is Unix seconds; `hosts` is the sorted, duplicate-free list of inventory names the challenge covers. Sent after `Assign` for every signed-mode run, and empty when every commit is signed. |
| R → S | `Started { run_id }` / `Refused { run_id, reason, detail }` | `reason` is one of the `run_hosts.refusal` values (data-model.md). |
| R → S | `Event { run_id, seq, json }` | One `keel_events` line (research R3). |
| R → S | `Log { run_id, seq, stream, bytes }` | Batched every 100 ms or 64 KiB, whichever comes first. |
| S → R | `Cancel { run_id }` | The executor sends SIGINT to `ansible-playbook`; after 30 s the runner stops the executor unit, and systemd kills its cgroup. |
| R → S | `Finished { run_id, exit_code, stats, ledger_line }` | `ledger_line` is the hash-chained line the executor appended (research R19). Keel stores it in `host_revisions`. |
| both | `Ping` / `Pong` | Every 15 s. After 3 missed pings a running host becomes `lost`. |

## Local socket (`/run/keel-runner/executor.sock`)

- Owned by root, group `keel-runner`, mode `0660`; `Accept=yes`, so each connection starts one
  `keel-executor@.service` instance as `keel-exec`.
- The network side forwards `Assign`, `Pack`, `Secrets`, `Approvals` and `Cancel` unchanged, and
  relays `Started`, `Refused`, `Event`, `Log` and `Finished` back. It does not interpret them, and
  the executor trusts nothing it says.
- `ReadLedger` returns the ledger head and per-job bases, for `Hello`.

## Guarantees

- Secrets travel only in `Secrets`, inside the SSH channel, and then over the local socket. They
  are written only into the run's private directory at `0600` and removed with it. Neither half
  logs them.
- The executor runs a job only after the checks in contracts/signed-mode.md pass (signed mode),
  or after the base, profile and replay checks (any mode).
- The host holds no database credentials, no master key, no receiver secrets, and no SSH key to
  any other host.
- The server writes `run_events` and `run_log_chunks` in batches, and forwards them live to
  subscribed clients. `seq` makes a reconnecting client's `SubscribeRun { from_seq }` resume
  without gaps.
