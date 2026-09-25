# Quickstart: Cluster Linter

1. `cd ui && pnpm install` (or symlink `node_modules`), then `pnpm dev` / `pnpm tauri dev`.
2. Connect to a cluster with a known-bad object, e.g. `kubectl run naked --image=nginx` (naked pod, no limits).
3. Open the **Linter** tab (dock tab on desktop, "Linter" stack tab on tablet/phone).
4. Findings fill in progressively; filter by severity/category; click a finding to open the Inspector on the object.
5. Verify: `cargo test -p fathom --test protocol_serde`, `cd ui && pnpm typecheck && pnpm lint && pnpm test`,
   `pnpm exec playwright test us1-linter-findings --project=<chromium project>`.
