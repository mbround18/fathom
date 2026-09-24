# Quickstart: 018-rbac-aware-mutation-ui

1. `cargo test -p fathom --test protocol_serde` and `cargo test -p fathom --test export_bindings`.
2. `pnpm -C ui typecheck && pnpm -C ui lint && pnpm -C ui test`.
3. E2E (Chromium only): `pnpm -C ui exec playwright test us1-mutation-access-gating --project=chromium`.
4. Manual: connect with a read-only identity, open a Node: cordon/drain/taint buttons are disabled with a tooltip
   giving the API server's reason; with an admin identity nothing changes.
