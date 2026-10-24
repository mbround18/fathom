# Contract: Depths configuration

Every flag has an environment variable; flags win. Read once at start. Invalid configuration fails
start with a message naming the setting.

| Flag | Env | Default | Meaning |
|---|---|---|---|
| `--listen` | `DEPTHS_LISTEN` | `0.0.0.0:8080` | Bind address. Not Dockge's 5001, so both can run side by side during a trial (R14). |
| `--public-url` | `DEPTHS_PUBLIC_URL` (alias `ORIGIN_URL`) | required | External URL; used for redirect URIs, cookie and Origin checks. Must be `https://` unless `--insecure-dev`. |
| `--stacks-dir` | `DEPTHS_STACKS_DIR` (alias `DOCKGE_STACKS_DIR`) | `/opt/stacks` | Stacks root (R1). |
| `--data-dir` | `DEPTHS_DATA_DIR` | `/var/lib/depths` | Audit log and `users.json`. Created 0700. |
| `--static-dir` | `STATIC_DIR` | `/usr/share/depths/web` | Built web assets, served as files. |
| `--owner-user-id` | `DEPTHS_OWNER_USER_ID` (alias `OWNER_USER_ID`) | none | Comma-separated owners, always admin (below). |
| `--docker-bin` | `DEPTHS_DOCKER_BIN` | `docker` | CLI to invoke. `DOCKER_HOST` is passed through (socket proxy). |
| — | `DEPTHS_OIDC_ISSUER`, `DEPTHS_OIDC_CLIENT_ID`, `DEPTHS_OIDC_CLIENT_SECRET` (or `_FILE`), `DEPTHS_OIDC_SCOPES` (default `openid profile email`) | — | OIDC provider. |
| — | `DEPTHS_GITHUB_CLIENT_ID`, `DEPTHS_GITHUB_CLIENT_SECRET` (or `_FILE`) | — | GitHub OAuth App. |
| — | `DEPTHS_DISCORD_CLIENT_ID`, `DEPTHS_DISCORD_CLIENT_SECRET` (or `_FILE`) | — | Discord OAuth2 application (R13). Redirect URI `<public_url>/auth/callback/discord`. |
| — | `OAUTH_PROVIDER` (`oidc`\|`github`\|`discord`), `OAUTH_CLIENT_ID`, `OAUTH_CLIENT_SECRET` (or `_FILE`) | — | Generic aliases for one provider's client credentials (R14). `OAUTH_CLIENT_*` without `OAUTH_PROVIDER` refuses to start. |
| `--session-idle` / `--session-max` | `DEPTHS_SESSION_IDLE`, `DEPTHS_SESSION_MAX` | `8h`, `24h` | Session lifetime. |
| `--max-file-bytes` | `DEPTHS_MAX_FILE_BYTES` | `1MiB` | Largest compose or `.env` file the editor loads. |
| `--audit-rotate-bytes` | `DEPTHS_AUDIT_ROTATE_BYTES` | `64MiB` | Rotation size. |
| — | `DEPTHS_FEATURE_SHELL`, `DEPTHS_FEATURE_EDIT` | `false`, `true` | Runtime flags (R12). The shell is off unless turned on (least privilege); turning it on also needs `DEPTHS_SOCKET_EXEC=1` on the socket proxy. |
| `--shell-policy` | `DEPTHS_SHELL_POLICY` | `strict` | 051 FR-007: `strict` (escape: nobody; root: admins) or `relaxed` (escape: admins; root: per role). Any other value refuses to start. |
| `--compose-policy` | `DEPTHS_COMPOSE_POLICY` | `warn` | 051 FR-017: `warn` (findings shown and audited) or `enforce` (escape findings refuse a `user`'s Up/Update/ServiceUp/Save/Create; an admin's preview is `ack_required`). Any other value refuses to start. |
| `--host-proc` | `DEPTHS_HOST_PROC` | unset = off | The host's `/proc`, mounted read-only (e.g. `/proc:/host/proc:ro` → `/host/proc`). Turns on admin-only host metrics (`WatchHost`): CPU, load, memory, swap, uptime, and disk of the stacks directory. Must be an absolute directory with a readable `stat`, or Depths refuses to start. Exposes the host's process list and command lines to the Depths process. |
| — | `OTEL_EXPORTER_OTLP_ENDPOINT` (+ standard `OTEL_*`) | unset = off | Telemetry export. |
| `--network-gate`, `--network-maturity-days` | `DEPTHS_NETWORK_GATE`, `DEPTHS_NETWORK_MATURITY_DAYS`, `DEPTHS_NETWORK_V4_PREFIX`, `DEPTHS_NETWORK_V6_PREFIX`, `DEPTHS_TRUSTED_NETWORKS`, `DEPTHS_GEOIP_DB` | `true`, `7`, `32`, `64`, empty, unset | Network trust and the restricted view: see spec 052 `contracts/config.md`. |
| — | `DEPTHS_FEATURE_FILES`, `DEPTHS_HELPER_IMAGE`, `DEPTHS_FILES_*` | `full`, own image, see 053 | The volume explorer's mode, helper image and limits: see spec 053 `contracts/config.md`. |
| `--insecure-dev` | `DEPTHS_INSECURE_DEV` | `false` | Allows `http://` public URL and non-`__Host-` cookie. Refused when `DEPTHS_LISTEN` is not loopback. |

At least one provider must be configured; with none, Depths refuses to start (FR-028: no
anonymous mode). Secrets are accepted via `*_FILE` and never logged.

Aliases (R14) rank below Depths' own names: flag > `DEPTHS_*` > alias > default. `ORIGIN_URL` with no
scheme is read as `https://`. Start-up logs which name each setting came from.

## Owner and users (`users.json`)

```sh
# .env on the host: one Discord owner (bare value = OAUTH_PROVIDER's id space)
OAUTH_PROVIDER=discord
OWNER_USER_ID=140642000969007104
# or several, provider-qualified
OWNER_USER_ID=discord:140642000969007104,github:mbround18
```

- Ids are `discord:<snowflake>`, `github:<login>` or `github:<numeric id>`, `oidc:<sub>` (matched
  against the configured issuer). A bare value belongs to `OAUTH_PROVIDER`; a bare value with no
  `OAUTH_PROVIDER` refuses to start.
- A `github:<login>` owner is pinned to the account's numeric id at first sign-in and matched by id
  from then on (a renamed or re-registered login does not inherit it).
- Owners are always `admin`, on every stack. The UI and protocol cannot demote or remove them.
- Everyone else lives in `<data-dir>/users.json`, written only by Depths (write to a temp file,
  fsync, rename; mode 0600). It is not meant to be hand-edited; a file that fails to parse refuses
  start with its path and the error, never an empty registry.
- Anyone who signs in is recorded as `pending` (identity, first and last seen) with no access.
  An admin gives them `user` (with a list of stack names) or `admin`, or pre-adds an id that has
  never signed in.
- Start-up refuses, naming `OWNER_USER_ID`, when no owner is configured and `users.json` holds no
  admin.

## Example compose (`apps/depths/compose.yaml`)

Shape only; the file is the deliverable.

- `socket-proxy`: `lscr.io/linuxserver/socket-proxy:3.4.7`, env as in R8, `/var/run/docker.sock`
  mounted read-only, `read_only: true`, `tmpfs: /run`, internal network only,
  `restart: ${RESTART_POLICY:-no}`.
- `depths`: the image; `DOCKER_HOST=tcp://socket-proxy:2375`; `${STACKS_DIR:-/opt/stacks}` mounted
  at the **same path** inside; `depths-data` volume at `/var/lib/depths` (audit log, `users.json`);
  `user: "10001:10001"`; port `8080`;
  `restart: ${RESTART_POLICY:-no}`; `env_file: .env` for the provider secrets and `OWNER_USER_ID`.
- The file's header comment says plainly that the socket proxy narrows but does not remove
  root-equivalent access.
