# Networks and the restricted view

Depths remembers the networks each person signs in from. On a network it does not trust yet, you
get a **restricted view**: enough to look around and to start, stop or restart a stack for
maintenance, but nothing that changes files, removes things or opens a shell. A stolen session
used from somewhere new can therefore do very little.

## Which networks are trusted

A network is one address for IPv4 and a `/64` for IPv6, so a phone that changes its address
inside the same home connection stays on one network.

- **Your first network.** The network you use the very first time you sign in is trusted.
- **Every other network is unknown** until something below trusts it. Depths asks you once:
  *Is this your home network?*
  - **Home**: the network becomes trusted after a waiting period (7 days by default), and only
    once you have used it on at least two separate days. The answer alone never trusts it.
  - **Public** (a café, a hotel, a friend's Wi-Fi): it stays restricted for good.
  - **Not now**: nothing is saved, and Depths asks again next time.
- **Whitelisted by an admin**: trusted at once (see [People and access](users.md)). An admin can
  only whitelist from a network that is already trusted.
- **Listed in `DEPTHS_TRUSTED_NETWORKS`**: always trusted, for everyone. Use it for your LAN or
  your tailnet (`100.64.0.0/10`).

You can change your answer at any time on **Your sessions** (the person icon in the top bar).
Changing to *home* starts the waiting period; it still does not trust the network straight away.

Locations shown next to networks are approximate and for your information only. They never
decide trust.

## What the restricted view allows

| Allowed on any network | Only on a trusted network |
|---|---|
| Viewing stacks, containers, logs and stats | Down, pull and update |
| Viewing compose files and masked `.env` values | Revealing `.env` values |
| Start, stop and restart (a stack or one service) | Editing and creating stacks |
| Answering the home or public question | Deleting stacks |
| Ending your own sessions | Container shells |
|  | The audit log and its export |
|  | Managing people and whitelisting networks |

The server enforces this, not the page: a refused command is answered with `untrusted_network`
and recorded in the audit log as `network_refused`, with the network and the command. Your normal
permissions still apply first; the restricted view never grants anything.

## Settings

| Flag | Environment | Default | What it does |
|---|---|---|---|
| `--network-gate` | `DEPTHS_NETWORK_GATE` | `true` | With `false`, every network is trusted. Networks are still recorded and shown |
| `--network-maturity-days` | `DEPTHS_NETWORK_MATURITY_DAYS` | `7` | Days before a *home* network is trusted. `0` means home networks are never trusted on their own |
| | `DEPTHS_NETWORK_V4_PREFIX` | `32` | How IPv4 addresses are grouped into one network |
| | `DEPTHS_NETWORK_V6_PREFIX` | `64` | How IPv6 addresses are grouped into one network |
| | `DEPTHS_TRUSTED_NETWORKS` | empty | Comma-separated CIDRs that are always trusted |
| | `DEPTHS_GEOIP_DB` | unset | A City `.mmdb` file for approximate locations. See [Installing Depths](install.md#approximate-locations) |

A CIDR or prefix that does not parse stops start-up with a message naming the setting.

## Behind a reverse proxy

Trust depends on Depths seeing each person's real address. Behind a proxy, set
[`DEPTHS_TRUSTED_PROXIES`](install.md#depths_trusted_proxies). If it is missing, every request
seems to come from the proxy: the first person's first network is then the proxy, and everyone is
trusted through it. Depths notices when every address it sees is private and shows admins a
notice in the top bar until you fix it.

## Getting back in

If you can no longer reach Depths from any trusted network (you moved, your provider changed your
address, or every whitelist entry is gone), add the network you are on now to
`DEPTHS_TRUSTED_NETWORKS` in the `.env` file and restart Depths:

```sh
DEPTHS_TRUSTED_NETWORKS=198.51.100.7/32
docker compose up -d
```

Sign in, whitelist the network for yourself on the Users page, then remove the line and restart
again if you want it trusted only for you.

## Where it is stored

Networks are kept in `networks.json` in the data directory, next to `users.json`, written the same
way (a new file renamed over the old one, mode 0600). Do not edit it by hand. Deleting it makes
everyone's next network their *first* network again.

# Containers and shells

Depths rates every container in your stacks by how far a process inside it is from the host, and
uses that rating to decide who may open a shell in it. Stacks should run **rootless**: a container
that runs as root on a daemon that runs as root is root on the host the moment something breaks
out of it.

## The three levels

| Level | Shown as | What it means |
|---|---|---|
| ok | nothing | Not root on the host, and nothing below that reaches it |
| root | amber **root** chip | The container runs as uid 0 (or declares no user) on a rootful daemon, and nothing worse |
| escape | red **escape** chip | The container can reach the host: a shell in it is a shell on the host |

A container is **escape** when any of these is true:

- it runs `privileged`;
- it mounts the container runtime's socket (`docker.sock`, `containerd.sock`, rootless ones under
  `/run/user/<uid>` included), **even read-only**: read-only stops writes to the socket file, not
  API calls through it;
- it mounts the host's `/`, or `/proc`, `/sys`, `/dev`, `/boot`, `/root`, `/var/lib/docker`,
  `/var/lib/containerd` or the kernel modules **writable** (or `/etc`, `/home`, `/run` or
  `/var/run` themselves, writable);
- it adds `SYS_ADMIN`, `SYS_MODULE`, `SYS_RAWIO`, `DAC_READ_SEARCH` or `BPF` (or `ALL`);
- it shares the host's pid, ipc or user namespace, or is given host devices;
- it runs as root with seccomp or AppArmor turned off (`unconfined`).
- it is a public **tunnel agent** on the host's network (`network_mode: host`), see below.

The host's `/` or `/etc` mounted **read-only** is rated **root**, not escape: it shows the
host's files (user names, configuration) but cannot change them. Other sensitive paths mounted
read-only are informational. So are other added capabilities (`NET_ADMIN` among them) and the
host's network, which widen what a container can do on the network without handing it the host.

### Tunnel agents (playit.gg, Cloudflare Tunnel, ngrok, ...)

A tunnel agent makes an outbound connection to a service on the internet. That service then
forwards public traffic back through the connection. It is how a game server or a web app gets
a public address without opening a port on the router: the firewall never sees the traffic
arrive, because the agent fetches it from inside.

Depths recognises playit.gg (`playit-cloud/playit-agent`), Cloudflare Tunnel (`cloudflared`),
ngrok, frp (`frpc`), bore, zrok, Pinggy and localtunnel by the image's name.

- **On the host's network** (`network_mode: host`) a tunnel agent is rated **escape**. It can
  reach every port on the host, including services that listen only on `localhost` because they
  were never meant to be public: the Docker API on a TCP port, databases, admin panels, the Depths
  socket proxy. What it publishes is decided in the tunnel provider's dashboard, not on this
  machine. So anyone who signs in to that playit.gg (or Cloudflare, ngrok, ...) account, or steals
  the agent's secret key, can put any of those ports on the internet, past the firewall.
- **On a container network** it is informational: it can still publish what that network
  reaches, but not the host's own services.

To bring one down to informational, give it its own network and list only the services it should
expose:

```yaml
services:
  playit:
    image: ghcr.io/playit-cloud/playit-agent:1.0
    user: "1000:1000"
    networks: [public]
  game:
    networks: [public]
networks:
  public: {}
```

In the playit.gg dashboard, the tunnel's local address then becomes `game:<port>` instead of
`127.0.0.1:<port>`. Keep the agent's secret key in `.env` and treat it like a password.

Root on a **rootless** or **userns-remapped** daemon is rated ok: uid 0 inside maps to an
unprivileged user outside. The chip's tooltip (hover it, focus it with the keyboard, or tap it on
a phone) lists every reason, worst first. A stack in the list shows the worst level of its
containers.

Admins see a notice on the overview when the daemon itself runs as root, linking here.

## Who may open a shell

The server checks the container **at the moment you ask for a shell**, with a fresh
`docker inspect`, not the rating the page last showed. If that inspect fails, the shell is
refused (`shell_unverifiable`): Depths never opens a shell into a container it could not check.
Your role and stack assignments are checked first, as always; the level only ever narrows them.

`DEPTHS_SHELL_POLICY` decides the rest:

| Container | `strict` (default) | `relaxed` |
|---|---|---|
| ok | as your role allows | as your role allows |
| root | admins only | as your role allows |
| escape | **nobody** | admins only |
| could not be checked | nobody | nobody |

Every shell opened, and every shell refused, is recorded in the audit log with the container's
level and reasons. A shell ends when its container is removed or recreated (a redeploy makes a
new container under the same name, and the shell does not follow it), and a shell open only
because its person is an admin ends when they stop being one.

Shells always start as the container's own user, with no `--privileged`, and the shell path is
checked before it is passed to `docker exec`.

## Compose lint

Before an `up`, an update, a service start, a save or a new stack, Depths reads the compose file
and lists what weakens containment, with the same levels and reasons as above, each with its
service and line. The editor shows the same findings as you type. The file is read as written:
`${VAR}` is judged at its worst, and `extends` and `include` are not followed.

Depths never changes your file. `DEPTHS_COMPOSE_POLICY` decides whether findings block:

| Policy | root findings | escape findings |
|---|---|---|
| `warn` (default) | shown | shown, and recorded in the audit log with the operation |
| `enforce` | shown | a `user` is refused (`compose_blocked`); an admin must tick *Deploy it anyway*, recorded as acknowledged in the audit log |

Stop, restart, down and pull do not deploy the file, so they are not linted.

## Running stacks rootless

In order of preference:

1. **Rootless Docker.** The whole daemon runs as an ordinary user, so root in any container is
   that user on the host. See Docker's
   [rootless mode](https://docs.docker.com/engine/security/rootless/) guide. Point Depths'
   socket proxy at the rootless socket (`$XDG_RUNTIME_DIR/docker.sock`).
2. **userns-remap.** The daemon stays root, but containers' uid 0 maps to a range of unprivileged
   uids. Set `"userns-remap": "default"` in `/etc/docker/daemon.json` and restart the daemon; see
   Docker's [user namespace](https://docs.docker.com/engine/security/userns-remap/) guide.
   Existing volumes may need their ownership changed.
3. **A `user:` in each service**, which works on any daemon:

   ```yaml
   services:
     app:
       image: nginx:1.29
       user: "1000:1000"
   ```

   Many images also publish a non-root variant (`nginx-unprivileged`, `-rootless` tags). Give the
   user ownership of anything the service writes to.

Depths reads the daemon's mode from `docker info` and rates root containers on rootless and
userns-remapped daemons as ok.

## What this does not cover

- **The socket proxy does not filter exec.** With `DEPTHS_SOCKET_EXEC=1`, the proxy lets exec
  calls through whatever their body asks for: a client that reaches the proxy directly could
  exec as root, or with `Privileged` set, in any container. Depths itself never asks for either,
  and the proxy is on an internal network only Depths can reach. Keep it that way, and leave exec
  off unless you use shells.
- **Docker API access is still root on the host.** A person who can edit a compose file under
  `warn` can deploy a privileged container. `enforce` closes that for `user`s; admins are trusted
  with root by design (see [Installing Depths](install.md#read-this-first-docker-access-is-root)).
- **Ratings are read from `docker inspect`**, cached by container id for display and re-read
  for every shell. A container changed behind Depths' back is caught at the next shell request,
  not before.
