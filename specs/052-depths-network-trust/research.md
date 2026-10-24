# Research: Depths network trust

## R1 Network key
- **Decision**: IPv4 /32 and IPv6 /64 by default (`DEPTHS_NETWORK_V4_PREFIX`, `DEPTHS_NETWORK_V6_PREFIX`); IPv4-mapped IPv6 normalised to IPv4.
- **Rationale**: privacy addresses rotate within a /64; home IPv4 is usually a single address.

## R2 Trust rule (pure)
`trusted(now, key, person_networks, settings) = gate_off || in(always_trusted) || src==first || whitelisted || (answer==home && now - first_used >= window && window>0 && distinct_days>=2)`. `public` never matures. Days counted in UTC.

## R3 First network
- **Decision**: the first sign-in after the feature exists, per person, sets `source=first` atomically under the registry lock (two simultaneous first sign-ins: the first persisted wins). Audited as `network.first`.

## R4 Where the gate runs
- **Decision**: one `gate(session, client_ip, cmd_kind)` called in WS dispatch after `permits` and in every mutating HTTP route; restricted commands = view/subscribe kinds, `Start`, `Stop`, `Restart`, `AnswerNetwork`, `EndSession(s)`. Evaluated per command with the command's own connection address (WS address fixed at upgrade; a new address means a new socket).
- **Shell interplay**: on an untrusted request from a session with open shells, close them (FR-012).

## R5 Revocation
- **Decision**: `SessionStore::end` already broadcasts `Ended`; ws.rs closes sockets on it (exists). Reuse; add test for ≤5 s.

## R6 Geo-IP
- **Decision**: `maxminddb` reading DB-IP City Lite or GeoLite2 City; city/region/country; private/loopback/CGNAT(100.64/10)/ULA show "private network"; failures logged once at start.

## R7 Proxy warning
- **Decision**: with the gate on and `DEPTHS_TRUSTED_PROXIES` empty, if the first 20 authenticated requests all come from private addresses, log a warning once and set `proxy_warning` for admins in `/api/me`.
