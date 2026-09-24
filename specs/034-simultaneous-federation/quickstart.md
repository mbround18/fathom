# Quickstart: Simultaneous Federation

1. Have two reachable contexts (e.g. kind clusters `kind-a`, `kind-b`).
2. Launch Fathom, open the Fleet dialog, Federation section, and toggle both contexts federated.
3. Both rows go Pending -> Live; the combined header shows 2/2 clusters.
4. `kubectl --context kind-b run bad --image=nope/nope`: within 5s kind-b's unhealthy count and the combined total rise; kind-a and the active context are unchanged.
5. Stop kind-b's control plane: its row shows Unreachable with the verbatim error while kind-a keeps updating. Restart it: the row recovers.
6. Restart Fathom: both contexts are restored as federated.
7. Untoggle kind-a: its row disappears and its client/tasks are released.
8. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `pnpm -C ui lint`, `typecheck`, `test`, Playwright federation specs. Validate on Ubuntu 26.04, Windows 11 and macOS 26 (Tier 1).
