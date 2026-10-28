# Contract: join tokens, join requests and runner downloads

A second way to enrol a host, beside the one-time token (contracts/runner.md, "Enrollment"). An
operator generates a **join token** in Keel. It is a signed JWT that carries everything a host
needs to find and pin this Keel, so the host's Ansible vars shrink to that one value plus the
trust decisions that belong in the repository. A host that presents a join token does not get
in: it becomes a **join request** that an operator accepts or rejects in the UI, after comparing
the host key fingerprint with the one the host printed.

The token answers "who is knocking": it says which workspace a request is for and which Keel
issued it, and it keeps strangers from filling the request list. It grants nothing on its own.
Leaking it costs a bounded number of pending requests, which rotation clears.

## Join token

A JWS compact serialization (a JWT), signed by Keel, readable with any JWT tool.

- **Header**: exactly `{"alg":"EdDSA","typ":"keel-join+jwt","kid":"<key id>"}`. The server
  rejects any other `alg`, `typ`, a missing `kid`, or extra header fields (no `jku`, `jwk`,
  `x5u`, `crit`), so `none` and algorithm confusion are impossible. No JWT library is needed:
  `ed25519-dalek` and `base64` are already dependencies.
- **Signing key**: a dedicated Ed25519 key per Keel instance, generated at first start and sealed
  in the database with the master key, like the SSH CAs. It signs nothing else. `kid` is the
  SHA-256 of its public key, truncated to 16 bytes, hex.
- **Claims** (all required unless marked):

  | Claim | Meaning |
  |---|---|
  | `iss` | Keel's public URL (`KEEL_PUBLIC_URL`). |
  | `aud` | `keel-runner`. |
  | `sub` | Workspace id. |
  | `jti` | Token id, 16 random bytes, hex. Rotation and revocation work by `jti`. |
  | `iat`, `exp` | Issue and expiry, seconds. `exp` is chosen at issue, default 30 days, at most 365. |
  | `keel.server` | `host:port` of the runner listener (from `KEEL_RUNNER_HOST_NAMES` and the runner port). |
  | `keel.host_ca` | The full `@cert-authority` public key line, as `keel ssh-ca show` prints it. |
  | `keel.origin`, `keel.rp_id` | Keel's web origin and passkey RP id, for `policy.toml`. |
  | `keel.inventory_name` (optional) | When set, the request is pre-named; the operator can still change it on accept. |
  | `keel.profiles` (optional) | Profiles to prefill on accept. Advisory: the host's own `allowed_profiles` is the ceiling. |

- **Not secret, but a bearer value.** Anyone can read the claims (they hold no secrets), and the
  signature makes them unforgeable, so the token itself is the credential. It is shown once on
  issue, stored only as its `jti` plus a SHA-256 of the whole token, and never logged, audited,
  exported or returned again.
- **Active token**: one per workspace, or none. **None is the default**: until an operator issues
  one, the join path refuses everything. The server accepts a token only if its signature
  verifies, `iss`/`aud` match, it is unexpired, and its `jti` is the workspace's active token.
- **Rotate** issues a new token and retires the old `jti` at once, with no grace period. Every
  pending request made with the old one is rejected (reason `token_rotated`), because rotation is
  what an operator does after a leak. Hosts already accepted are unaffected: after enrolment a
  host authenticates with its own key and certificate, never with the join token.
- **Disable** retires the active `jti`; pending requests are rejected the same way.
- Rotating Keel's join signing key (`PreviewJoinSigningKeyRotate`, step-up) invalidates every
  token at once, for the case where the key itself is suspect.
- Issue, rotate and disable need step-up and are audited (by `jti`, never the token).

### What the host takes from the token, and what it must not

The runner cannot verify the signature (it has no prior trust in Keel); it trusts the token
because the operator put it there, exactly as it trusts `keel_runner_host_ca` today. So the token
supplies **connection settings only**: `server`, `host_ca`, `origin`, `rp_id`. Trust decisions
stay in the repository's vars and are never read from the token: `allowed_profiles`,
`require_signed`, signers, approvers and the override quorum. A server-issued value must not be
able to widen what a host accepts.

If a var and the token both give a connection setting (for example `keel_runner_host_ca` is set
and differs from `keel.host_ca`), the role fails rather than picking one.

On hosts the token is an Ansible var, `keel_join_token`, kept in the repository's encrypted vars.
It is multi-use and expires, so unlike `keel_enroll_token` it may live there.

## Runner side (`keel-join-v1` subsystem)

1. The base profile decodes the token's claims (no signature check: see above), writes
   `runner.env` (`KEEL_RUNNER_SERVER`), `host_ca.pub` and the connection keys of `policy.toml`
   from them, writes the token to `/var/lib/keel-runner/join-token` (`0400`, owner
   `keel-runner`) with `no_log: true`, and starts the service. If both an enrollment token and a
   join token are present, the enrollment token wins and the join token is deleted after
   enrolment.
2. The runner refuses to connect if the token is expired or if `keel.host_ca` differs from
   `host_ca.pub`.
3. It authenticates with its host key (unknown to Keel, so only the join and enroll subsystems are
   open) and sends:

   ```rust
   JoinRequest {
       token: String,
       inventory_name: String,     // policy.toml; a suggestion, the operator may change it
       allowed_profiles: Vec<String>,
       require_signed: bool,
       hostname: String,           // from the OS, at most 253 bytes
       os: String,                 // /etc/os-release PRETTY_NAME, at most 128 bytes
       arch: String,               // x86_64 | aarch64
       runner_version: String,
   }
   ```

4. The server replies one of:
   - `JoinPending { request_id }`: the runner closes, waits, and asks again with the same frame.
     Back-off: 30 s, doubling to 10 min, with jitter. The key fingerprint ties the retries to one
     request; a retry never creates a second one.
   - `Enrolled { host_id, cert }`: as for an enrollment token. The runner deletes `join-token`
     and continues as an enrolled host.
   - `JoinRejected { reason }`: the runner deletes `join-token`, logs the reason, and exits with
     status 78 (`EX_CONFIG`). The unit sets `RestartPreventExitStatus=78`, so a rejected host does
     not knock in a loop. Re-applying the profile with a (new) token starts over.
5. `keel-runner fingerprint` prints the host key's SHA-256 fingerprint and exits. The runner also
   logs it once at start while not enrolled, and the role prints it, so the operator can compare
   it with what the UI shows.

Each failed check (bad signature, retired or expired token, a malformed frame) closes the
connection and is audited by source address, key fingerprint and `jti` if readable, never with
the token.

## Server rules

- A request is unique per (workspace, host key fingerprint). The same key asking again updates
  `last_seen_at` and the reported facts; it does not reset the decision.
- A key fingerprint that already belongs to an enrolled or revoked host is refused (`known_key`).
- A rejected fingerprint is answered `JoinRejected` for 30 days without creating a new request.
- **Bounds** (fail closed when exceeded): at most 50 pending requests per workspace
  (`--join-pending-max`), further requests answered `JoinRejected { reason: "queue_full" }`
  without being stored; at most 10 join attempts per source address per minute
  (`--join-rate`), excess connections closed before any database write. Pending requests expire
  after 7 days (`expired`).
- Every frame field is length-bounded and the strings are shown escaped in the UI. Reported
  facts are the host's claims, labelled as such; only the key fingerprint is proof of anything.

## Accepting and rejecting (web protocol)

| Command (reply) | Rules |
|---|---|
| `ListJoinRequests { workspace, state? }` (`JoinRequests`) | Operators of the workspace. Shows fingerprint, reported facts, source address, first and last seen, state, and the `jti` it came with. |
| `PreviewJoinAccept { request, inventory_name, profiles }` | Step-up. `inventory_name` must be free in the workspace; `profiles` must be within the request's `allowed_profiles`. On confirm the host is created exactly as by an enrollment token (`hosts` row, certificate issued at the runner's next attempt) and the request becomes `accepted`. |
| `PreviewJoinReject { request, comment? }` | No step-up. The request becomes `rejected`. |
| `GetJoinToken { workspace }` (`JoinToken { active, jti, expires_at, created_at, created_by }`) | Never returns the token. |
| `PreviewJoinTokenIssue { workspace, expires_in, inventory_name?, profiles? }`, `PreviewJoinTokenRotate { workspace, expires_in }` | Step-up. On confirm, replies `JoinTokenSecret { token }` once. The rotate preview names the pending requests it will reject. |
| `PreviewJoinTokenDisable { workspace }` | Step-up. Names the pending requests it will reject. |
| `PreviewJoinSigningKeyRotate` | Owner, step-up. Every workspace's token stops working. |

Pushed event: `JoinRequested { request }`, to operators of the workspace, so the Hosts page shows
a new request without a reload.

UI: the Hosts page gets a **Join requests** tab (accept with inventory name and profiles
prefilled, reject, the fingerprint shown in full beside the instruction to compare it with
`keel-runner fingerprint` on the host). **Settings → Hosts** holds the join token: issue (with
expiry), rotate, disable, the one-time display with copy, and its decoded claims so the operator
can see what a host will be told.

## Runner downloads

Keel's image carries the runner release for the version it was built from, so a host can fetch
the binaries from the Keel it will join.

| Path | Response |
|---|---|
| `GET /runner/latest/SHA256SUMS` | Checksums of every file below, `sha256sum` format. |
| `GET /runner/latest/{arch}/keel-runner`, `.../keel-executor` | Static (musl) binaries. `arch` is `x86_64` or `aarch64`; anything else is `404`. |
| `GET /runner/latest/requirements.txt` | The hash-pinned `ansible-core` requirements. |
| `GET /runner/latest/keel_events.tar.gz` | The callback plugin. |
| `GET /runner/version` | `{ "version": "…", "commit": "…" }`. |

- `latest` means "what this server ships"; it is the only version until releases are cut from
  git tags. Then `/runner/{version}/…` serves the same files under the tag's version, and
  `latest` stays an alias. The path shape does not change.
- Served from `--runner-dist-dir` (`KEEL_RUNNER_DIST_DIR`, default `/usr/share/keel/runner`), from
  disk, never embedded. A missing directory makes every `/runner/` path `404`; the server still
  starts.
- **Public, no authentication**: the files are not secret, and gating them on the join token
  would put that token into every download log. Integrity does not come from the server.
- **Hosts never trust the server's checksums.** The role installs a file only if its SHA-256
  matches `keel_runner_sha256` pinned in the repository's vars. The operator takes the sums from
  the image they chose to run (`docker run --rm mbround18/keel:latest cat
  /usr/share/keel/runner/SHA256SUMS`), not from a live server, so a server compromised later
  cannot change what hosts install. Once releases are signed, the role verifies a signature from
  an offline release key instead.
- **No self-update.** The runner never downloads or replaces itself. Upgrades happen only when the
  operator changes the pinned version and checksums and re-applies the profile. A server that
  could push binaries would undo signed mode (FR-035).
- `Cache-Control: no-cache` and a strong `ETag` (the file's SHA-256).

## Tests

- Token: header strictness (`alg: none`, `HS256`, extra fields, wrong `typ`, missing `kid` all
  refused); bad signature; wrong `iss`/`aud`; expired; retired `jti`; a token from another Keel.
- Join: no active token → refused; host CA mismatch (runner refuses before connecting); retry
  creates no second request; accept issues a certificate on the next attempt; reject → exit 78
  and no restart; rotate rejects pending and refuses the old token; a known or revoked key is
  refused; the queue bound and rate limit hold; the token never appears in logs, audit or
  exports (extend T132).
- Downloads: each file served with the right checksum; unknown arch and path traversal are `404`;
  a missing dist dir does not stop the server.
- Playwright: a request appears live, accept with step-up, reject; issue, rotate (token shown
  once, claims decoded) and disable in Settings.
