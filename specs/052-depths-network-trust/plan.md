# Implementation Plan: Depths network trust and sessions

**Branch**: `050-depths-stack-manager` | **Date**: 2026-10-24 | **Spec**: [spec.md](spec.md)

## Summary

Record each person's networks, decide per request whether the request's network is trusted for its
sender with a pure rule in `depths-core`, and when it is not, allow only the restricted command set.
Add the home/public question, a sessions page with revocation, per-person known networks, an
admin whitelist from the users page, optional offline geo-IP, and a proxy-misconfiguration warning.

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19
**Primary Dependencies**: existing stack; new: `maxminddb` (offline .mmdb reader; licence ISC, check deny.toml)
**Storage**: `networks.json` under the data dir using the same atomic write as `users.rs` (temp 0600, fsync, rename, fsync dir); session metadata (address, browser) in memory in depths-server keyed by session handle, cleared on `Ended`
**Testing**: `cargo test -p depths-core` (trust rule with injected clock), `cargo test -p depths-server` (gate per command, revocation closes sockets, sweep), vitest, Playwright `us5-access`
**Constraints**: no outside calls; location never in decisions; web-auth (shared with Fathom) unchanged unless unavoidable
**Scale/Scope**: ≤ 64 networks per person, ≤ a few hundred people

## Constitution Check

- **I.** Whitelisting/forgetting are explicit, audited, trusted-network only. PASS
- **II.** Rule in core, enforcement in server, UI mirrors via `/api/me` trust state and refusal kinds. PASS
- **III.** Clock-injected table tests, per-command server tests, Playwright. PASS
- **V.** Recording throttled to once per hour per session; geo lookups cached in memory. PASS
- **VI.** Bounded, pruned storage; no location text in audit. PASS

## Project Structure

```text
crates/depths-core/src/network.rs             # NetworkKey, KnownNetwork, trust(), maturity, grouping
crates/depths-core/src/role.rs                # restricted_ok(cmd kind) table
apps/depths/server/src/networks.rs            # persisted registry (users.rs pattern), recording, whitelist
apps/depths/server/src/session_meta.rs        # address + browser per session handle
apps/depths/server/src/geoip.rs               # optional maxminddb reader
apps/depths/server/src/{config,ws,routes,client_ip}.rs   # settings, gate in dispatch, endpoints, warning
crates/depths-protocol/src/…                  # trust state, sessions, networks, refusal kind
apps/depths/web/src/features/sessions/…       # Your sessions page, question dialog, restricted banner
apps/depths/web/src/features/users/…          # admin networks + whitelist
docs/guides/depths/{security,users,install}.md
```

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| `maxminddb` dependency | locations offline as the user asked | external API: leaks addresses |
| session side table | web-auth is shared with Fathom | adding fields to web-auth Session: wider blast radius |
