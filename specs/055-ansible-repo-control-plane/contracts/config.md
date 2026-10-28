# Contract: configuration and deployment

## `keel` (server)

Each flag can also be set through the environment variable in parentheses.

| Flag (env) | Default | Notes |
|---|---|---|
| `--listen` (`KEEL_LISTEN`) | `0.0.0.0:7830` | UI, `/api`, `/hooks`. Put TLS in front, or set `--tls-cert/--tls-key`. |
| `--tls-cert` (`KEEL_TLS_CERT`) | unset | PEM certificate chain, leaf first. Set with `--tls-key`, and `--listen` serves HTTPS (rustls, TLS 1.2 and 1.3, HTTP/1.1). One without the other refuses start. Unset: plain HTTP, for a TLS proxy in front or `--insecure-dev`. |
| `--tls-key` (`KEEL_TLS_KEY`) | unset | PEM private key (PKCS#8, PKCS#1 or SEC1) for `--tls-cert`, read once at start, so it may be `0400`. A missing, unreadable or mismatched key refuses start. Errors name the file, never its contents. |
| `--runner-listen` (`KEEL_RUNNER_LISTEN`) | `0.0.0.0:7831` | The embedded SSH server for host runners (R17). Both SSH CAs are generated at first start and sealed in the database. Expose it only on the network hosts use (for example Tailscale). |
| `--runner-cert-ttl` | `24h` | Lifetime of runner certificates. |
| `--runner-max-offline` | `30d` | How long after certificate expiry a host may still renew with its bare key. |
| `--join-pending-max` | `50` | Pending join requests per workspace (contracts/join.md). |
| `--join-rate` | `10/min` | Join attempts per source address. |
| `--runner-dist-dir` (`KEEL_RUNNER_DIST_DIR`) | `/usr/share/keel/runner` | Runner binaries served at `/runner/latest/…`. Missing means `404`, not a start failure. |
| `--signed-mode-default` | `false` | Signed mode for new bindings. |
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
- `keel ssh-ca show`, which prints the CA public keys and the `@cert-authority` line for
  `keel_runner_host_ca`.

## Host runner

Installed by the base profile (contracts/host-install.md). Both binaries read
`/etc/keel-runner/trust/policy.toml` and accept no flags that weaken it.

| Binary | Flag (env) | Default | Notes |
|---|---|---|---|
| `keel-runner` | `--server` (`KEEL_RUNNER_SERVER`) | from `policy.toml` | `host:port`. |
| `keel-runner` | `--state-dir` | `/var/lib/keel-runner` | Host key, certificate, enrollment token. |
| `keel-runner` | `--executor-socket` | `/run/keel-runner/executor.sock` | |
| `keel-runner` | `keel-runner fingerprint` | | Prints the host key's SHA-256 fingerprint, to compare with a join request. |
| `keel-executor` | `--state-dir` | `/var/lib/keel-executor` | Git store, ledger, run directories. |
| `keel-executor` | `--venv` | `/opt/keel-runner/venv` | The only Python and Ansible it uses. |
| `keel-executor` | `keel-executor ledger verify` | | Checks the ledger chain, for operators. |

## Compose (`apps/keel/compose.yaml`)

- Services:
  - `keel`: server image, read-only root, non-root.
  - `keel-db`: PostgreSQL, current stable major.
- Every service has `restart: ${RESTART_POLICY:-no}`.
- Networks:
  - `edge`: `keel` only; publishes the web port and the runner SSH port.
  - `db`: `keel` and `keel-db`; internal.
- There is no runner service: runners live on the managed hosts.
