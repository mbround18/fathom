# Quickstart: Setup and navigation

## Try it

1. Point `KUBECONFIG` at a file with several contexts and no `current-context`, then start Fathom. One setup screen shows every context. The top bar and status bar are still there.
2. Filter the list, then pick a context. The sign-in step appears if issuers are configured; press **Skip**. The Overview opens.
3. Restart Fathom. The same context opens directly, and the kubeconfig file is unchanged (`git diff` or `sha256sum` it).
4. In the header, press **Settings** (just left of the sign-in chip) and choose **Cluster settings**. It holds only per-context settings, including the daemon switch with its explanation.
5. Look at the dock. It shows only **Logs** and **Events**, plus **Cluster history** when the daemon is connected.
6. In the navigator, open **Cluster › Topology** to see the whole-cluster graph. Open a Deployment, choose the inspector's **Topology** tab, and press **Follow** on a Pod, then on its Node.
7. Open **Integrations**. Certificates, Knative and KEDA are listed (greyed if not installed). Velero appears only when it is installed.
8. Open **Access Control › RoleBindings** and press **New Role…**. In **New RoleBinding**, choose **Create a new role…** in the role picker, create it, and confirm the binding form returns with the role filled in.
9. Hover any navigator entry to see its one-line description.

## Tests

```bash
pnpm -C ui test -- setupState navModel layout-defaults resourceTopology rememberedContext veleroVisibility
cd ui && pnpm exec playwright test tests/e2e/setup-screen.spec.ts tests/e2e/setup-sign-in.spec.ts \
  tests/e2e/settings-home.spec.ts tests/e2e/daemon-explained.spec.ts tests/e2e/dock-and-nav.spec.ts \
  tests/e2e/topology-sections.spec.ts tests/e2e/integrations-home.spec.ts \
  tests/e2e/role-discoverability.spec.ts --workers=4
```

Also run the updated existing specs listed in research.md ("Existing tests touched") with `--workers=6` or fewer when other agents are active.
