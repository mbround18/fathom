# Contract: Depths protocol (v1)

Defined in `crates/depths-protocol` (Rust, `serde` + `ts-rs`); TypeScript generated into
`apps/depths/web/src/protocol/generated/` and drift-checked in `scripts/ci.sh`. Envelope and
version handling mirror Fathom's `BackendCmdEnvelope` / `BackendEventEnvelope`.

## Transport

- `GET /api/v1/ws` upgrades to a WebSocket. Requires the session cookie and an `Origin` equal to
  `DEPTHS_PUBLIC_URL`'s origin. First client frame is `Hello{protocol_version, csrf_token}`; a
  mismatched token closes with **4403**, a missing/expired session with **4401**, too many sockets
  per session (default 8) with **4429** (same codes as Fathom hosted).
- Client → server frames: `CmdEnvelope{id, cmd}`. Server → client: `EventEnvelope{reply_to?, event}`.
- Server sends `SessionExpiring{at}` 60 s before expiry; the client refreshes via HTTP.

## Commands

| Command | Role | Result event(s) |
|---|---|---|
| `ListStacks` | user (filtered to assigned stacks; `foreign` and `standalone` empty) or admin | `Stacks{root, stacks, foreign, standalone, warnings, stale, as_of}` then live `StacksChanged`; `RuntimeUnavailable{reason}` when docker fails (FR-010) |
| `GetStack{name}` | user (assigned stack) | `StackDetail{stack, compose_text, env_text_masked \| env_text, content_hash}` |
| `RevealEnv{name}` | admin | `EnvRevealed{name, env_text}` (audited; masking is by key: names containing `PASS`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE`, `CREDENTIAL`, case-insensitive) |
| `PreviewOp{stack, kind, options}` | per data-model table | `Preview{…}` or `Denied` / `Busy` / `Invalid` |
| `PreviewSave{draft, deploy}` | user (assigned stack) for a save; admin for create | `Preview{file_diff, steps: [validate, up?]}`, or `ValidationFailed{errors[{file, line?, message}]}`, or `Conflict{current_text, current_env, current_hash}` |
| `Confirm{preview_id}` | as previewed | `OpStarted{operation_id}`, `OpOutput{operation_id, chunk}`*, `OpFinished{operation_id, outcome}` |
| `Cancel{operation_id}` | actor or admin | `OpFinished{outcome: cancelled}` (SIGINT to the compose process) |
| `OpenLogs{target, tail, since?}` | user (assigned stack; a `{container}` target outside a stack is admin) | `StreamOpened{stream_id}`, `LogChunk{stream_id, lines}`*, `StreamClosed{stream_id, reason}` |
| `OpenShell{container, shell?, cols, rows}` | user (assigned stack), flag on | `StreamOpened`, `ShellOutput{stream_id, bytes_b64}`*, `StreamClosed` |
| `ShellInput{stream_id, bytes_b64}` / `ShellResize{stream_id, cols, rows}` | owner of stream | none |
| `WatchStats{targets}` / `UnwatchStats` | user (assigned stack) | `StatsSample{samples}` every 2 s while watched |
| `WatchHost` / `UnwatchHost` | admin (`view_host`), and `DEPTHS_HOST_PROC` set | `HostSample{at, cpu_percent, cpu_cores, load{one, five, fifteen}, memory{total_bytes, available_bytes, used_bytes}, swap{total_bytes, free_bytes, used_bytes}, uptime_secs, disk{total_bytes, used_bytes, free_bytes}, containers_running, containers_stopped}` every 2 s while watched. Any figure the server cannot read is `null`. A non-admin gets `Denied{not_permitted}` whatever the config; an admin with the setting unset gets `Denied{feature_disabled}`. The watch takes one stream slot and ends when the person stops being an admin. |
| `CloseStream{stream_id}` | owner | `StreamClosed` |
| `GetAudit{filter{stack?, actor?, action?, from?, to?}, cursor?}` | admin | `AuditPage{entries, next_cursor?}` |
| `GetUsers` | admin | `Users{users, pending}`, then live `UsersChanged{users, pending}` |
| `PutUser{id, role, stacks}` | admin | `Users{…}` (audited `user_added` or `user_changed`); `Denied` for the owner, or for a role other than `user`/`admin` |
| `RemoveUser{id}` | admin | `Users{…}` (audited `user_removed`); `Denied` for the owner |

"user (assigned stack)" means role `user` with the stack in their `stacks`, or `admin`. `PreviewOp`
follows the data-model table. `id` in `PutUser`/`RemoveUser` is provider-qualified
(`discord:<snowflake>`, `github:<numeric id>`, `oidc:<sub>`); `PutUser` on an unknown id is an
invite.

`target` = `{stack, service}` (compose logs) or `{container}` (docker logs). Streams that drop
(proxy idle cut, R8) send `StreamClosed{reason: disconnected}`; the client reopens with backoff and
the tab shows "reconnected".

## Rules the server enforces (not the client)

1. Every command is authorised against the session's identity and the users registry as it is at
   the time of the command (FR-032, FR-033). Removing a person, or their last stack, ends their
   sessions and closes their sockets with **4403**.
2. `Confirm` succeeds only if the preview id exists, is unused, belongs to this session, has not
   expired, and the stack's current `content_hash` equals the one bound into it. Otherwise
   `PreviewVoid{reason}`.
3. One running operation per stack (`Busy{stack, operation_id, kind, actor}`).
4. All paths come from stack names resolved by the fs adapter; no command carries a path.
5. Each `Confirm`, denial, shell open and env reveal writes one audit entry before the event is
   sent.

## Container security (051)

Ratings, refusals and lint added by spec 051. Every field is additive and optional on the wire.

| Type | Shape |
|---|---|
| `SecurityLevel` | `ok \| root \| escape` |
| `SecuritySeverity` | `info \| root \| escape` |
| `SecurityReason` | `{code, severity, message}`; codes are snake_case (`privileged`, `runtime_socket`, `host_root_mount`, `sensitive_mount`, `sensitive_mount_read_only`, `dangerous_capability`, `added_capability`, `host_pid`, `host_ipc`, `host_userns`, `host_network`, `host_uts`, `unconfined`, `devices`, `root_user`, `root_remapped`, `no_user`, `interpolated`) |
| `ShellRule` | `per_role \| admin_only \| refused`: the policy applied to the level, past the role check |
| `ContainerSecurity` | `{level, reasons (worst first), shell: ShellRule}` on each `StackContainer.security`; absent when not yet inspected |
| `Stack.security` | the worst container level; absent when none is rated |
| `ComposeWarning` | `{service, code, severity, message, line}`; `line` 1-based or `null` |
| `Preview.warnings`, `Preview.ack_required` | findings for Up/Update/ServiceUp/Save/Create; `ack_required` only under `enforce` with an escape finding (admins only reach it) |
| `Features.shell_policy`, `Features.compose_policy` | `strict \| relaxed`, `warn \| enforce` |
| `Features.daemon` | `rootless \| userns \| rootful`, admins only; absent for anyone else or when unread |
| `AuditEntry.security` | `{level, reasons ("service:code" for lint), acknowledged}` on shell opens, shell refusals, denials and linted operations |

Commands and events:

| Command | Who | Answer |
|---|---|---|
| `LintCompose{stack, compose_text}` | admin (any name, so a new stack can be linted), or a `user` for an assigned stack | `LintResult{stack, findings: ComposeWarning[]}`. Never writes |
| `Confirm{preview_id, acknowledge_findings?}` | as previewed | An `ack_required` preview without `acknowledge_findings: true` from an admin is `Denied{compose_blocked, detail}` and **stays usable**; with it, the run is audited `acknowledged: true` |
| `OpenShell{…}` | as before, then the container gate | After the role check, a fresh `docker inspect` of that one container and the pure `shell_decision(policy, level, admin)`. `Denied{shell_escape \| shell_root_admin_only \| shell_unverifiable, detail}`; an inspect error or no match is `shell_unverifiable` (fail closed) |

`Denied` gains an optional `detail` (one sentence: the reasons, or the blocking findings).
`StreamClosed.reason` gains `container_recreated` and `container_removed` (051 FR-014): a shell
ends when its container id leaves the inventory or its name comes back under a new id. An
admin-only shell (root under `strict`) is closed `failed` when its person stops being an admin.

## HTTP endpoints

| Method + path | Auth | Purpose |
|---|---|---|
| `GET /healthz` | none | Liveness: `ok`. |
| `GET /readyz` | none | Docker reachable and stacks root readable; 503 otherwise with a reason. |
| `GET /auth/login?provider=oidc\|github\|discord` | none | Redirect to the provider (state, nonce, PKCE). |
| `GET /auth/callback/{provider}` | none | Completes sign-in, sets cookie, redirects to `/`. |
| `POST /auth/refresh` | session + CSRF header | Revalidates the provider token (OIDC refresh, Discord refresh token, GitHub `GET /user`); a rejected token ends the session. |
| `POST /auth/logout` | session + CSRF header | Ends session; RP-initiated logout URL for OIDC. |
| `GET /api/v1/me` | session | `{identity, csrf_token, role: user\|admin\|pending, owner, stacks, features{shell, edit, host_metrics}}`; `host_metrics` is true only for an admin when `DEPTHS_HOST_PROC` is set; `pending` drives the no-access page. |
| `GET /api/v1/audit/export?…` | session (admin) | Filtered JSONL download. |
| `GET /*` | none | Static files from `--static-dir`, SPA fallback to `index.html`. |

All responses carry `Content-Security-Policy` (self only, `connect-src` self + ws), 
`X-Content-Type-Options`, `Referrer-Policy: same-origin`, `X-Frame-Options: DENY`. Mutating HTTP
requests require `X-CSRF-Token` matching the session (`web-auth::csrf::check_mutation`).
