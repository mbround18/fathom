# Contract: Depths configuration

Every flag has an environment variable; flags win. Read once at start (the roles file is also
watched). Invalid configuration fails start with a message naming the setting, except the roles
file (below).

| Flag | Env | Default | Meaning |
|---|---|---|---|
| `--listen` | `DEPTHS_LISTEN` | `0.0.0.0:5001` | Bind address (Dockge's port, so a swap needs no port change). |
| `--public-url` | `DEPTHS_PUBLIC_URL` | required | External URL; used for redirect URIs, cookie and Origin checks. Must be `https://` unless `--insecure-dev`. |
| `--stacks-dir` | `DEPTHS_STACKS_DIR` | `/opt/stacks` | Stacks root (R1). |
| `--data-dir` | `DEPTHS_DATA_DIR` | `/var/lib/depths` | Audit log. Created 0700. |
| `--static-dir` | `STATIC_DIR` | `/usr/share/depths/web` | Built web assets, served as files. |
| `--roles-file` | `DEPTHS_ROLES_FILE` | `/etc/depths/roles.toml` | Role mapping. |
| `--docker-bin` | `DEPTHS_DOCKER_BIN` | `docker` | CLI to invoke. `DOCKER_HOST` is passed through (socket proxy). |
| — | `DEPTHS_OIDC_ISSUER`, `DEPTHS_OIDC_CLIENT_ID`, `DEPTHS_OIDC_CLIENT_SECRET` (or `_FILE`), `DEPTHS_OIDC_SCOPES` (default `openid profile email groups`), `DEPTHS_OIDC_GROUPS_CLAIM` (default `groups`) | — | OIDC provider. |
| — | `DEPTHS_GITHUB_CLIENT_ID`, `DEPTHS_GITHUB_CLIENT_SECRET` (or `_FILE`) | — | GitHub OAuth App. |
| `--session-idle` / `--session-max` | `DEPTHS_SESSION_IDLE`, `DEPTHS_SESSION_MAX` | `8h`, `24h` | Session lifetime. |
| `--max-file-bytes` | `DEPTHS_MAX_FILE_BYTES` | `1MiB` | Largest compose or `.env` file the editor loads. |
| `--audit-rotate-bytes` | `DEPTHS_AUDIT_ROTATE_BYTES` | `64MiB` | Rotation size. |
| — | `DEPTHS_FEATURE_SHELL`, `DEPTHS_FEATURE_EDIT` | `true`, `true` | Runtime flags (R12). |
| — | `OTEL_EXPORTER_OTLP_ENDPOINT` (+ standard `OTEL_*`) | unset = off | Telemetry export. |
| `--insecure-dev` | `DEPTHS_INSECURE_DEV` | `false` | Allows `http://` public URL and non-`__Host-` cookie. Refused when `DEPTHS_LISTEN` is not loopback. |

At least one provider must be configured; with none, Depths refuses to start (FR-028: no
anonymous mode). Secrets are accepted via `*_FILE` and never logged.

## roles.toml

```toml
# Highest matching grant wins. Groups are the IdP's group claim values,
# or github:<org> / github:<org>/<team-slug> for GitHub sign-in.
[[grant]]
group = "homelab-admins"
role = "admin"
stacks = "*"

[[grant]]
group = "github:acme/media"
role = "operator"
stacks = ["jellyfin", "arr-*"]

[[grant]]
group = "family"
role = "viewer"
stacks = ["jellyfin"]
```

- `role` ∈ `viewer | operator | admin`; `stacks` is `"*"` or a list of names or `*`-globs.
- Unknown keys or values are errors.
- On change the file is re-read. If it fails to parse, the previous mapping stays in force, the
  error is logged and shown to admins (`GetRoles.last_error`). If it is missing or invalid at first
  start, no one has any role (fail closed) and the log says why.

## Example compose (`apps/depths/compose.yaml`)

Shape only; the file is the deliverable.

- `socket-proxy`: `lscr.io/linuxserver/socket-proxy:3.4.7`, env as in R8, `/var/run/docker.sock`
  mounted read-only, `read_only: true`, `tmpfs: /run`, internal network only,
  `restart: ${RESTART_POLICY:-no}`.
- `depths`: the image; `DOCKER_HOST=tcp://socket-proxy:2375`; `${STACKS_DIR:-/opt/stacks}` mounted
  at the **same path** inside; `depths-data` volume at `/var/lib/depths`; `./roles.toml`
  read-only at `/etc/depths/roles.toml`; `user: "10001:10001"`; port `5001`;
  `restart: ${RESTART_POLICY:-no}`; `env_file: .env` for the provider secrets.
- The file's header comment says plainly that the socket proxy narrows but does not remove
  root-equivalent access.
