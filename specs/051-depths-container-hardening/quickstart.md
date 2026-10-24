# Quickstart: validate container hardening

1. `CARGO_BUILD_JOBS=4 nice cargo test -p depths-core security compose_lint` — rating, policy and lint tables pass.
2. `CARGO_BUILD_JOBS=4 nice cargo test -p depths-runtime-cli -p depths-server` — inspect parsing, argv (no `--privileged`/`--user`), guard refusals audited, sweep rows.
3. `DEPTHS_E2E_PORT=4196 nice pnpm --filter @companion/depths-web exec playwright test --project=us4-shells` — chips render with reasons; escape shell refused with reason; admin sees root shell.
4. Manual (dev daemon): run `busybox` with `--privileged` in a test stack → `escape` chip, shell refused under strict, allowed for admin under relaxed; plain `busybox` with `user: "1000"` → `ok`.
5. Edit a compose with `privileged: true` → lint warning; with `DEPTHS_COMPOSE_POLICY=enforce` → save blocked for user, admin acknowledgement succeeds and appears in the audit log; file bytes unchanged by Depths.
