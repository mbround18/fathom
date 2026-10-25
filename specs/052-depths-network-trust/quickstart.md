# Quickstart: validate network trust

1. `CARGO_BUILD_JOBS=4 nice cargo test -p depths-core network` — rule boundaries: first, whitelist, always, home matures at exactly the window with 2 days, public never, 0 disables maturity, /64 grouping.
2. `CARGO_BUILD_JOBS=4 nice cargo test -p depths-server` — with injected clock and peer: each FR-010 command refused and audited from an untrusted network; start/stop/restart allowed; end-session allowed; whitelist refused from untrusted; revoked session's socket closes within 5 s; sweep rows.
3. `DEPTHS_E2E_PORT=4196 nice pnpm --filter @companion/depths-web exec playwright test --project=us5-access` — question dialog, restricted banner, disabled controls with reason, sessions page end-others, admin whitelist.
4. Manual: run with `DEPTHS_TRUSTED_PROXIES` set to the reverse proxy; confirm Your sessions shows the real client address; unset it and confirm the warning.
5. Run with no network access and `DEPTHS_GEOIP_DB` set: locations appear, no outbound connections.
