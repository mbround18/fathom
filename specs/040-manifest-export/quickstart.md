# Quickstart / validation: 040 Manifest Export

## Automated

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                       # incl. manifest_export + kubeconfig_export unit tests
cargo test -p fathom --test export_bindings  # then: git diff --exit-code ui/src/protocol/generated
pnpm -C ui typecheck && pnpm -C ui lint && pnpm -C ui test
pnpm -C ui exec playwright test tests/e2e/manifest-export.spec.ts --project=chromium --project=webkit
```

The e2e suite covers: Inspector Copy (clean and raw), Download filename and content, the row-menu Export dialog, bulk export of three rows as multi-doc, per-object error listing, the RoleBinding review manifest view with copy, kind config preview, and kubeconfig export (redacted preview, embedded flag, confirm step, result path; hidden in hosted).

## Live re-apply check (kind only; NEVER the `default` context)

1. Run Fathom against `kind-fathom-dev` (desktop, or the `ws` dev bridge).
2. Open a Deployment (e.g. `kube-system/coredns`), YAML tab, clean on, Download. Or run the backend fetch path through the dev bridge.
3. `kubectl --context kind-fathom-dev apply --dry-run=server -f deployment-kube-system-coredns.yaml`. Expected: `deployment.apps/coredns configured (server dry run)` (or `unchanged`).
4. Kubeconfig: export `kind-fathom-dev` to a scratch path, then `stat -c %a` shows `600`, and `kubectl --kubeconfig <file> get ns` works.
