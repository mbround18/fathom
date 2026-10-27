# Contract: configuration and deployment

## `keel` (server)

Each flag can also be set through the environment variable in parentheses.

| Flag (env) | Default | Notes |
|---|---|---|
| `--listen` (`KEEL_LISTEN`) | `0.0.0.0:7830` | UI, `/api`, `/hooks`. Put TLS in front, or set `--tls-cert/--tls-key`. |
| `--runner-listen` (`KEEL_RUNNER_LISTEN`) | `0.0.0.0:7831` | Bind only on the internal runner network. |
| `--public-url` (`KEEL_PUBLIC_URL`) | required | Used for receiver URLs, OIDC redirects and cookie scope. |
| `--database-url` (`KEEL_DATABASE_URL`) | required | PostgreSQL. Migrations run at start, and are additive only. |
| `--master-key-file` (`KEEL_MASTER_KEY_FILE`) | required | 32 random bytes, host-owned, `0400`. Keel refuses to start if it is missing. Create it with `keel keygen`. |
| `--data-dir` (`KEEL_DATA_DIR`) | `/var/lib/keel` | Git mirrors and export downloads. |
| `--static-dir` (`KEEL_STATIC_DIR`) | `/usr/share/keel/web` | Web `dist`, served from disk and never embedded. |
| auth flags | none | The same `web-auth` provider flags as Depths (OIDC, GitHub, Discord). At least one is required; there is no anonymous mode. |
| `--step-up-window` | `5m` | |
| `--approval-ttl-default` | `24h` | |
| `--run-log-cap` | `64MiB` | |
| `--otel-endpoint` (`OTEL_EXPORTER_OTLP_ENDPOINT`) | unset | Telemetry stays off unless this is set. |

## Subcommands

- `keel keygen`
- `keel audit verify`, which checks the hash chain.
- `keel recover-passkeys`, as in 054, run on the host.
- `keel migrate`, which applies database migrations without serving.

## `keel-runner`

| Flag (env) | Default | Notes |
|---|---|---|
| `--server` (`KEEL_SERVER`) | required | `ws://keel:7831`, on the internal network. |
| `--enroll-token-file` | none | Needed only for first enrollment. |
| `--state-dir` | `/var/lib/keel-runner` | The runner key and the pinned server fingerprint. |
| `--job-uid` / `--job-gid` | `10002` | R2 |

## Compose (`apps/keel/compose.yaml`)

- Services:
  - `keel`: server image, read-only root, non-root.
  - `keel-runner`: runner image, `cap_drop: [ALL]`, `cap_add: [SETUID, SETGID, KILL]`,
    `read_only: true`, `tmpfs: /work`, `security_opt: [no-new-privileges:true]`, pids and memory
    limits.
  - `keel-db`: PostgreSQL, current stable major.
- Every service has `restart: ${RESTART_POLICY:-no}`.
- Networks:
  - `edge`: `keel` only.
  - `runners`: `keel` and `keel-runner`; internal.
  - `db`: `keel` and `keel-db`; internal.
  - The runner also gets egress to managed nodes and to Git (none here: the server fetches).
- The runner has no route to `keel-db`.
