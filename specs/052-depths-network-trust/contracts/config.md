# Config additions (052)

| Flag / env | Default | Meaning |
|---|---|---|
| `--network-gate` / `DEPTHS_NETWORK_GATE` | `true` | off ⇒ every network trusted (still recorded) |
| `--network-maturity-days` / `DEPTHS_NETWORK_MATURITY_DAYS` | `7` | 0 ⇒ "home" networks never mature on their own |
| `DEPTHS_NETWORK_V4_PREFIX` / `_V6_PREFIX` | `32` / `64` | grouping |
| `DEPTHS_TRUSTED_NETWORKS` | empty | comma CIDR list, always trusted (LAN, tailnet `100.64.0.0/10`) |
| `DEPTHS_GEOIP_DB` | unset | path to a City .mmdb; display only |

Invalid CIDRs or prefixes refuse to start, naming the variable.
