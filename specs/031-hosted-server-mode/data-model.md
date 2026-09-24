# Data Model: Hosted Server Mode

All server-internal types are Rust in `server/`; only fields crossing the browser boundary are
in `src/protocol/v1/` (ts-rs generated). Nothing below is persisted except AuditEntry.

## HostedConfig (server startup, validated fail-closed)

| Field | Type | Notes |
|---|---|---|
| `external_url` | Url (https) | Origin for cookies/CSRF/redirect URI `<external>/auth/callback`. |
| `oidc.issuer` | Url | Discovery must succeed at startup or the process exits non-zero (FR-014). |
| `oidc.client_id` / `client_secret` | String / SecretString | Secret from env/mounted file, never logged. |
| `oidc.scopes` | Vec<String> | default `openid profile email offline_access groups`. |
| `oidc.ca_file` | Option<Path> | Custom CA (kind overlay). |
| `claims.username` / `claims.groups` | String | defaults `preferred_username` / `groups`. |
| `claims.username_prefix` / `groups_prefix` | String | must match API server flags in pass-through. |
| `access_model` | `PassThrough` \| `Impersonation` | default PassThrough. |
| `kube_token` | `Id` \| `Access` | pass-through token kind, default `Id`. |
| `audit_viewer_groups` | Vec<String> | who may see all audit entries. |
| `limits` | SessionLimits | below. |
| `session.absolute` / `idle` | Duration | defaults 8 h / 30 min. |
| `state_dir` | Path | default `/var/lib/fathom`. |

Validation: https external URL (except `--allow-insecure-localhost` for dev, refused with a
non-loopback host), non-empty client id, discovery reachable, `impersonation` requires an SA
token present, `replicas` n/a.

## Identity (User Identity)

`{ sub: String, username: String, groups: Vec<String>, email: Option<String> }`; derived from
validated ID-token claims; `sub`+issuer is the stable key (`user_key = sha256(iss|sub)[..16]`).

## Session (User Session, in-memory only)

| Field | Notes |
|---|---|
| `id` | 256-bit random, base64url; cookie value. |
| `csrf` | per-session CSRF token. |
| `identity` | Identity above. |
| `tokens` | `TokenCell`: id_token, access_token, refresh_token (all `SecretString`), `expires_at`. |
| `created_at`, `last_seen`, `absolute_deadline` | lifetime enforcement. |
| `backend` | `Arc<UserBackend>` (shared per `user_key`). |
| `state` | `Active` -> `Refreshing` -> `Ended{reason}`; reasons: SignedOut, Expired, Revoked, IdpUnavailable, LimitEvicted. |

State transitions: sign-in creates Active; refresh success keeps Active; refresh permanent
failure, sweeper expiry or logout -> Ended (backend aborted, WS closed 4401).

## UserBackend

`{ user_key, link: BackendLink, hub: FanoutHub, thread: JoinHandle, limits: Counters,
subscribers: usize, last_activity }`; created lazily on first WS subscribe, dropped when no
sessions reference it after `reuseWindow`.

## SessionLimits (Session Limits)

`max_sessions` 50, `max_sessions_per_user` 3, `max_watches_per_user` 30,
`max_streams_per_user` 10, `max_tabs_per_user` 5, `stream_recycle_minutes` 4,
`idle_park_minutes` 10. All configurable; exposed read-only to the UI via `SubscriptionInfo.limits`.

## AccessModel

Enum only; selects the `ClusterAccess` constructor: PassThrough (bearer from `TokenCell`) or
Impersonation (SA token + `Impersonate-User`, one `Impersonate-Group` per group).

## Protocol additions (src/protocol/v1)

- `SubscriptionInfo` += `mode: Option<ShellMode>` (`Desktop|Hosted`, default absent = Desktop),
  `identity: Option<PublicIdentity {username, groups}>`, `limits: Option<PublicLimits>`.
- `SystemEvent` += `SessionEnded { reason: SessionEndReason }` and
  `LimitReached { kind, max }`.
- `AuditLogEntry` += `actor: Option<AuditActor { sub, username }>` (serde default; absent for
  desktop and legacy files).
- `BridgeError` unchanged; limit and filter refusals use `Forbidden(reason)`.

## AuditEntry (extended)

Existing fields (spec 026) + `actor`. Hosted file: `<state_dir>/audit/audit-log.jsonl`,
append-only, mode 0600, one writer mutex; loaders tolerate missing `actor`.
