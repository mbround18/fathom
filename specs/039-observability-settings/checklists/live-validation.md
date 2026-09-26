# Live validation: 039 Observability settings

- **Date**: 2026-09-26
- **Cluster**: `kind-fathom-dev` through the devbridge (`FATHOM_LIVE_UI_PORT=4391`)

## Quickstart (T024)

| Step | Result | Notes |
|---|---|---|
| 1 backend tests | PASS | 17/17. The quickstart's filter `observability` matched **0 tests**; it is corrected to `cargo test -p fathom --lib settings_tests` |
| 2 `pnpm -C ui test` | PASS | 1283/1283 |
| 3 fake e2e, phone + laptop, `observability-settings.spec.ts` | PASS | 8/8 |
| 4 live: save URL + key, reload, URL prefilled, "Key set", re-select the context | PASS | `-g observability` passed on `chromium-phone` and `chromium-wide`. The devbridge pins one context and uses the debug-only in-memory secret store, so live covers neither switching to a *different* context (isolation is covered by the `settings_are_isolated_per_context` unit test and the fake e2e) nor the real OS keyring |
| 5 secrecy grep (count only) | PASS | key `live-e2e-secret-key-039`: **0** in `~/.fathom/integrations.json`, **0** in the devbridge log at `RUST_LOG=debug` (2625 lines), **0** files under `~/.fathom`. The URL is present in `integrations.json` (1). The spec also asserts the key is in no page HTML and no received WebSocket frame |

## Gate (T023)

- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo test --workspace`: 921 passed, 0 failed.
- UI checks: typecheck pass; eslint 0 errors (27 warnings, all pre-existing and none in files touched here); prettier clean; vitest 1283/1283.
- Playwright on chromium-phone, tablet, laptop and wide, plus webkit-laptop: 1658 passed, 172 skipped, 5 failed. Their outcome on re-run:
  - Three passed serially: chromium-tablet taint, chromium-wide perf-10k, and webkit-laptop terminal. These were load flakes at 12 workers.
  - Two WebKit clipboard specs failed, because `grantPermissions(['clipboard-write'])` is unknown to WebKit. The harness is fixed, and they now pass on webkit and chromium.
- The remaining 9 projects (chromium desktop/ultra/ultra-plus, and webkit phone/tablet/desktop/wide/ultra/ultra-plus): 3085 passed, 208 skipped, 10 failed at 10 workers. All 10 pass serially.
