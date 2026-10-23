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
| `ListStacks` | viewer (filtered to stacks with a role) | `Stacks{root, stacks, foreign, standalone, warnings}` then live `StacksChanged` |
| `GetStack{name}` | viewer | `StackDetail{stack, compose_text, env_text_masked \| env_text, content_hash}` |
| `RevealEnv{name}` | operator | `EnvRevealed{name, env_text}` (audited; masking is by key: names containing `PASS`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE`, `CREDENTIAL`, case-insensitive) |
| `PreviewOp{stack, kind, options}` | per data-model table | `Preview{…}` or `Denied` / `Busy` / `Invalid` |
| `PreviewSave{draft, deploy}` | admin | `Preview{file_diff, steps: [validate, up?]}`, or `ValidationFailed{errors[{line?, message}]}`, or `Conflict{current_text, current_hash}` |
| `Confirm{preview_id}` | as previewed | `OpStarted{operation_id}`, `OpOutput{operation_id, chunk}`*, `OpFinished{operation_id, outcome}` |
| `Cancel{operation_id}` | actor or admin | `OpFinished{outcome: cancelled}` (SIGINT to the compose process) |
| `OpenLogs{target, tail, since?}` | viewer | `StreamOpened{stream_id}`, `LogChunk{stream_id, lines}`*, `StreamClosed{stream_id, reason}` |
| `OpenShell{container, shell?, cols, rows}` | operator, flag on | `StreamOpened`, `ShellOutput{stream_id, bytes_b64}`*, `StreamClosed` |
| `ShellInput{stream_id, bytes_b64}` / `ShellResize{stream_id, cols, rows}` | owner of stream | none |
| `WatchStats{targets}` / `UnwatchStats` | viewer | `StatsSample{samples}` every 2 s while watched |
| `CloseStream{stream_id}` | owner | `StreamClosed` |
| `GetAudit{filter{stack?, actor?, action?, from?, to?}, cursor?}` | admin | `AuditPage{entries, next_cursor?}` |
| `GetRoles` | admin | `Roles{grants, source_path, last_error?}` |

`target` = `{stack, service}` (compose logs) or `{container}` (docker logs). Streams that drop
(proxy idle cut, R8) send `StreamClosed{reason: disconnected}`; the client reopens with backoff and
the tab shows "reconnected".

## Rules the server enforces (not the client)

1. Every command is authorised against the session's identity and the roles mapping at the time of
   the command (FR-032).
2. `Confirm` succeeds only if the preview id exists, is unused, belongs to this session, has not
   expired, and the stack's current `content_hash` equals the one bound into it. Otherwise
   `PreviewVoid{reason}`.
3. One running operation per stack (`Busy`).
4. All paths come from stack names resolved by the fs adapter; no command carries a path.
5. Each `Confirm`, denial, shell open and env reveal writes one audit entry before the event is
   sent.

## HTTP endpoints

| Method + path | Auth | Purpose |
|---|---|---|
| `GET /healthz` | none | Liveness: `ok`. |
| `GET /readyz` | none | Docker reachable and stacks root readable; 503 otherwise with a reason. |
| `GET /auth/login?provider=oidc\|github` | none | Redirect to the provider (state, nonce, PKCE). |
| `GET /auth/callback/{provider}` | none | Completes sign-in, sets cookie, redirects to `/`. |
| `POST /auth/refresh` | session + CSRF header | Re-reads groups (GitHub teams / OIDC refresh). |
| `POST /auth/logout` | session + CSRF header | Ends session; RP-initiated logout URL for OIDC. |
| `GET /api/v1/me` | session | `{identity, csrf_token, roles_summary, features}`. |
| `GET /api/v1/audit/export?…` | session (admin) | Filtered JSONL download. |
| `GET /*` | none | Static files from `--static-dir`, SPA fallback to `index.html`. |

All responses carry `Content-Security-Policy` (self only, `connect-src` self + ws), 
`X-Content-Type-Options`, `Referrer-Policy: same-origin`, `X-Frame-Options: DENY`. Mutating HTTP
requests require `X-CSRF-Token` matching the session (`web-auth::csrf::check_mutation`).
