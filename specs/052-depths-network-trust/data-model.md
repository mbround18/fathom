# Data Model: Depths network trust

## KnownNetwork (persisted, per person, `networks.json`)
| field | type | notes |
|---|---|---|
| key | string | canonical CIDR, e.g. `203.0.113.7/32`, `2001:db8:1:2::/64` |
| first_used | RFC3339 | |
| last_used | RFC3339 | |
| days | sorted `YYYY-MM-DD`, max 8 kept | only "≥2 distinct" matters |
| source | `first \| answer \| whitelist \| seen` | |
| answer | `home \| public \| none` | |
| whitelisted_by / at / label | optional | label ≤ 64 chars, plain text |

File: `{ version: 1, people: { <user_id>: KnownNetwork[] } }`; ≤ 64 per person (LRU drop of non-whitelisted); non-whitelisted unused 180 days pruned at load and daily.

## SessionMeta (memory)
`{ handle, address, browser (≤ 80 chars, derived from User-Agent), last_recorded }`; removed on `Ended`.

## TrustDecision
`{ trusted: bool, reason: always|first|whitelist|matured|gate_off|unknown|public|maturing, matures_at?: RFC3339, network: key }`.

## Settings
`DEPTHS_NETWORK_GATE` (bool, true), `DEPTHS_NETWORK_MATURITY_DAYS` (7; 0 = never mature), `DEPTHS_NETWORK_V4_PREFIX` (32), `DEPTHS_NETWORK_V6_PREFIX` (64), `DEPTHS_TRUSTED_NETWORKS` (CIDR list), `DEPTHS_GEOIP_DB` (path, optional).

## Audit events
`network.first`, `network.seen`, `network.answer`, `network.whitelist.add|remove`, `network.forget`, `session.end {by, target}`, `network.refused {cmd, network}`.
