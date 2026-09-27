# Quickstart: Role and ClusterRole create/edit

## Try it

1. Connect to a cluster and open **Access Control → Roles**.
2. Press **New Role**, enter a name, keep the namespace or change it.
3. In the rule row, add `core` in API groups (the default) and pick `configmaps` in resources. The preset stays **Read-only**.
4. Press **Review**: the exact YAML is shown with Copy and Download. Press **Apply** and confirm.
5. Press **Bind this role…** to open the RoleBinding form pre-filled with the new role.
6. Open the role from the list and press **Edit** to change its rules. Review shows the diff.

## Tests

```bash
pnpm -C ui test -- roleYaml
cd ui && pnpm exec playwright test tests/e2e/rbac-role-create.spec.ts --workers=2
```

Live (kind `fathom-dev` only):

```bash
kind get kubeconfig --name fathom-dev > "$SCRATCH/kind-fathom-dev.kubeconfig"
cd ui && KUBECONFIG="$SCRATCH/kind-fathom-dev.kubeconfig" FATHOM_LIVE_CONTEXT=kind-fathom-dev \
  pnpm exec playwright test -c playwright.live.config.ts tests/live/rbac-role-create.spec.ts \
  --project chromium-wide --workers=1
```

The live spec deletes its `fathom-e2e-role-*` Role and `fathom-e2e-cr-*` ClusterRole in `afterEach`.
