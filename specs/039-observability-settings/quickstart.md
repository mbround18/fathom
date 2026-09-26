# Quickstart: Observability Settings

1. Backend: `cargo test -p fathom --lib settings_tests` (protocol round-trip, per-context isolation, legacy file load, blank-key keep, clear, keyring-failure path with the in-memory store).
2. UI unit: `pnpm -C ui test` (store handler, form hydration and indicators).
3. Fake e2e: `pnpm -C ui exec playwright test --project=chromium-phone --project=chromium-laptop tests/e2e/observability-settings.spec.ts --workers=2`.
4. Live (kind-fathom-dev): `pnpm -C ui e2e:live -g observability`. Save Grafana URL + key, reopen, confirm URL prefilled and key shown as set; switch context and confirm isolation.
5. Manual secrecy check: grep `~/.fathom/integrations.json` and the app log for the key value; expect no match.
