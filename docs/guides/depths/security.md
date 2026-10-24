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
