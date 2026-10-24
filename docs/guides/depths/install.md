# Installing Depths

Depths runs as one container. The example compose file in the repository,
[`apps/depths/compose.yaml`](../../../apps/depths/compose.yaml), runs it with a Docker socket proxy
in front of the Docker daemon. This guide walks through that file and every setting it uses.

## Read this first: Docker access is root

Anything that can talk to the Docker API can start a privileged container that mounts the host's
filesystem, so **access to the Docker API is root on the host**. Depths needs that access to run
your stacks. The socket proxy narrows which parts of the API Depths can reach, but it does not
remove that power: Depths must be able to create containers, and a person who can edit a compose
file can make one privileged.

So:

- Only admit people you would trust with root on this host.
- Only give the `user` role, and only assign stacks, to people you would trust with what those
  stacks can do. A user who can edit a stack's compose file can change what it mounts.
- Put Depths behind https, and never publish its port on a public interface.

## 1. Get the image

No registry image is published yet. Build it from a checkout of the repository:

```sh
make depths-image        # builds depths:dev
```

The image is a multi-stage build: the server and the web app are compiled in separate stages and
copied into a `debian:trixie-slim` runtime with the Docker CLI and the Compose plugin. It runs as
user 10001, and its health check calls `/healthz`.

To use an image you built and pushed somewhere yourself, set `DEPTHS_IMAGE` in `.env`.

## 2. Copy the compose file and the settings

Put `apps/depths/compose.yaml` and `apps/depths/.env.example` in a directory of their own on the
host, outside your stacks directory, and copy the settings file:

```sh
cp .env.example .env
```

Fill in `.env`:

```sh
# The https URL people open. Redirect URIs are <this>/auth/callback/<provider>.
DEPTHS_PUBLIC_URL=https://depths.example.com

# Where the stacks live on the host.
STACKS_DIR=/opt/stacks

# One sign-in provider (see sign-in.md).
OAUTH_PROVIDER=github
OAUTH_CLIENT_ID=<client id>
OAUTH_CLIENT_SECRET=<client secret>

# Your own id with that provider: admin from the first sign-in (see users.md).
OWNER_USER_ID=<your id>

# The reverse proxy's address, so the audit log records the real client.
DEPTHS_TRUSTED_PROXIES=

# Shells stay off until you turn on both of these.
DEPTHS_FEATURE_SHELL=false
DEPTHS_SOCKET_EXEC=0

RESTART_POLICY=no
```

Then start it:

```sh
docker compose up -d
```

`.env` holds your client secret. Keep it readable by its owner only (`chmod 600 .env`).

## What the compose file does

### The socket proxy

The `socket-proxy` service (`lscr.io/linuxserver/socket-proxy:3.4.7`) is the only thing that
mounts `/var/run/docker.sock`, and it mounts it read-only. Depths talks to it over
`DOCKER_HOST=tcp://socket-proxy:2375` on the `docker-api` network, which is internal: neither
service can reach anything else through it.

The proxy allows only the API areas Compose needs: containers, images, networks, volumes, info,
events, ping, version and logs, with `POST=1` (Compose needs it to create, start and remove
anything). Two areas are closed by default:

- **Exec** (`EXEC`), which container shells need. It follows `DEPTHS_SOCKET_EXEC`, default `0`.
- **Builds** (`BUILD`, `SESSION`, `GRPC`). If any of your stacks have a `build:` section, add
  `BUILD: 1`, `SESSION: 1` and `GRPC: 1` to the proxy's environment.

The proxy closes connections that sit idle for 10 minutes. Log tabs reconnect on their own, so
this shows up at most as a "Reconnected" note in a quiet log tab.

If you leave the proxy out and mount the socket into Depths directly, the container runs as user
10001, so it also needs the socket's group: add `group_add: ["<gid of /var/run/docker.sock>"]` and
remove the `DOCKER_HOST` line. That gives Depths the whole Docker API.

### The Depths container

- **Same-path stacks mount.** The stacks directory is mounted at the same path inside the
  container as on the host (`${STACKS_DIR}:${STACKS_DIR}`), and `DEPTHS_STACKS_DIR` is set to that
  path. Compose resolves relative paths in a stack, such as a `./data` bind mount, inside the
  Depths container, and the Docker daemon then mounts that path on the host. If the two paths
  differ, those bind mounts point at the wrong place. When Depths sees containers whose recorded
  project directory is under a different root, it shows a warning on that stack.
- **Data volume.** The `depths-data` volume at `/var/lib/depths` holds the audit log and
  `users.json` (the people you have admitted). Back it up; losing it means admitting everyone
  again.
- **Locked down.** A read-only root filesystem with a tmpfs `/tmp`, all capabilities dropped,
  `no-new-privileges`, a 512-process limit and 1 GB of memory.
- **Host metrics.** The host's `/proc` is mounted read-only at `/host/proc`, and
  `DEPTHS_HOST_PROC` points at it. See [Host metrics](#host-metrics) for what that exposes.
- **Loopback only.** The port is published as `127.0.0.1:${DEPTHS_PORT:-8080}:8080`, so only
  programs on the host can reach it.

### Restart policy

Both services use `restart: ${RESTART_POLICY:-no}`. With the default, nothing comes back after a
reboot or a crash until you run `docker compose up -d` again. On a server you want running all the
time, set `RESTART_POLICY=unless-stopped` in `.env`.

## 3. Put a reverse proxy in front

Depths only accepts an `https://` public URL, sets its session cookie as `Secure`, and checks the
`Origin` of every request against `DEPTHS_PUBLIC_URL`. Run a reverse proxy on the host (Caddy,
nginx, Traefik, or a VPN sidecar such as Tailscale that terminates TLS) that:

- serves `https://depths.example.com` with a valid certificate,
- forwards to `http://127.0.0.1:8080`,
- passes WebSocket upgrades on `/api/v1/ws` (the whole UI runs over that one socket), and
- does not time out idle WebSockets too quickly (log tabs can be quiet for a long time).

With Caddy, for example:

```
depths.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

### DEPTHS_TRUSTED_PROXIES

Depths records a source address with every audit entry and applies its rate limits per address.
Behind a proxy, the connection comes from the proxy, so Depths needs to know which addresses may
tell it the real client address.

`DEPTHS_TRUSTED_PROXIES` is a comma-separated list of CIDRs (a bare address counts as one host).

- When the connection comes from an address in that list, Depths uses `CF-Connecting-IP` if the
  request has one, and otherwise the right-most `X-Forwarded-For` entry that is not itself a
  trusted proxy.
- When the connection comes from anywhere else, both headers are ignored and the connecting
  address is the client. This is why the list is empty by default: a forged header from an
  untrusted peer changes nothing.

List the address Depths sees the proxy connect from. A proxy on the host that reaches the published
loopback port usually arrives from the gateway address of the compose project's `web` network
(`docker network inspect <project>_web`). If every audit entry shows the same address, that is the
proxy's, and it belongs in the list.

## Health checks

- `GET /healthz` answers when the process is up.
- `GET /readyz` answers `503` when the stacks directory is missing or the Docker API (through the
  proxy) does not answer, which is the first thing to check when stacks do not show up.

## All settings

Every setting can be given as a flag or an environment variable, and a flag wins. Depths' own
`DEPTHS_*` names win over the aliases, which exist so a Dockge `.env` carries over. A setting that
is wrong stops start-up with a message naming it, and the start-up log says where each setting
came from.

| Flag | Environment | Default | What it does |
|---|---|---|---|
| `--listen` | `DEPTHS_LISTEN` | `0.0.0.0:8080` | Address to listen on, inside the container |
| `--public-url` | `DEPTHS_PUBLIC_URL` (alias `ORIGIN_URL`) | required | The `https://` origin people open: no path. An `ORIGIN_URL` with no scheme is read as `https://` |
| `--stacks-dir` | `DEPTHS_STACKS_DIR` (alias `DOCKGE_STACKS_DIR`) | `/opt/stacks` | The stacks directory. Must be an absolute path that exists |
| `--data-dir` | `DEPTHS_DATA_DIR` | `/var/lib/depths` | Audit log and `users.json`. Created with mode 0700 |
| `--static-dir` | `STATIC_DIR` | `/usr/share/depths/web` | The built web app |
| `--owner-user-id` | `DEPTHS_OWNER_USER_ID` (alias `OWNER_USER_ID`) | none | Owners, comma-separated: always admin. See [People and access](users.md) |
| `--docker-bin` | `DEPTHS_DOCKER_BIN` | `docker` | The Docker CLI to run. `DOCKER_HOST` is passed through to it |
| `--session-idle` | `DEPTHS_SESSION_IDLE` | `8h` | A session ends after this long without use |
| `--session-max` | `DEPTHS_SESSION_MAX` | `24h` | A session ends after this long, used or not. Must not be shorter than the idle limit |
| `--max-file-bytes` | `DEPTHS_MAX_FILE_BYTES` | `1MiB` | Largest compose or `.env` file the editor opens |
| `--audit-rotate-bytes` | `DEPTHS_AUDIT_ROTATE_BYTES` | `64MiB` | Audit log size at which it rotates |
| `--trusted-proxies` | `DEPTHS_TRUSTED_PROXIES` | empty | Proxies whose forwarding headers are believed (above) |
| `--insecure-dev` | `DEPTHS_INSECURE_DEV` | `false` | For development only: allows an `http://` public URL. Refused unless Depths listens on a loopback address |
| | `DEPTHS_FEATURE_SHELL` | `false` | Container shells; see below |
| | `DEPTHS_FEATURE_EDIT` | `true` | Editing compose and `.env` files. With `false`, Depths refuses to save edits |
| `--host-proc` | `DEPTHS_HOST_PROC` | unset | A read-only mount of the host's `/proc` (the compose file uses `/host/proc`). Turns on host metrics for admins. Must be an absolute directory with a readable `stat`. Unset means off; see below |
| | `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | Sends telemetry to an OpenTelemetry collector (the standard `OTEL_*` variables apply). Unset means no export |

Durations take `s`, `m`, `h` or `d` (`30m`, `8h`). Sizes take `b`, `KiB`, `MiB`, `GiB`, `KB`, `MB`
or `GB`. Booleans take `true`/`false`, `1`/`0`, `yes`/`no` or `on`/`off`.

The sign-in settings are in [Sign-in providers](sign-in.md). Every secret can also be given as a
file: set `NAME_FILE` to its path instead of `NAME` (setting both stops start-up). Secrets are
never logged.

These are read by the compose file, not by Depths:

| Variable | Default | What it does |
|---|---|---|
| `STACKS_DIR` | `/opt/stacks` | The host stacks directory, mounted at the same path |
| `DEPTHS_IMAGE` | `depths:dev` | The Depths image to run |
| `DEPTHS_PORT` | `8080` | The loopback port on the host |
| `DEPTHS_SOCKET_EXEC` | `0` | Opens exec on the socket proxy, for shells |
| `RESTART_POLICY` | `no` | Restart policy for both services |

## Turning on shells

A shell tab runs a command inside a container, which is another way to act as root on whatever
that container can reach. Shells need two switches, so neither one alone opens them:

1. `DEPTHS_FEATURE_SHELL=true`, which shows the shell button and lets Depths start a session.
2. `DEPTHS_SOCKET_EXEC=1`, which lets the socket proxy pass exec calls through.

Both default to off. If you run Depths without the proxy, the second switch does not exist, and
`DEPTHS_FEATURE_SHELL` alone decides.

Even with shells on, a `user` can only open one in a container of a stack assigned to them.

## Host metrics

Admins can see the host's CPU, memory, swap, disk, load and uptime on the overview, beside the
stack list. Everyone also sees CPU and memory for each stack they can view, which needs no extra
setting. On a phone, admins get a one-line host summary above the stack list.

Depths reads host figures from a copy of the host's `/proc`. The compose file mounts it read-only
at `/host/proc` and sets `DEPTHS_HOST_PROC=/host/proc`. Disk space is measured for the filesystem
that holds the stacks directory.

**The trade-off.** Depths reads only `stat`, `loadavg`, `meminfo` and `uptime`. The mount, though,
lets the Depths process read the host's whole process list, including every process's command
line. Command lines can contain secrets, such as a password passed as an argument. If Depths were
ever compromised, an attacker could read them. Depths already has root-equivalent access through
Docker, so this adds little for most hosts. If you would rather not take it on:

- remove the `/proc:/host/proc:ro` mount and the `DEPTHS_HOST_PROC` line from `compose.yaml`;
- with `DEPTHS_HOST_PROC` unset, the host panel is hidden and `WatchHost` is refused;
- per-stack figures still work, because they come from Docker.

Host figures are only ever sent to admins. A `user` is refused whether the setting is on or off,
and an admin who is demoted stops receiving them straight away.
