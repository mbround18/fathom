# Prod scoped agent: RBAC set up through the 011 UI

**Date**: 2026-09-26
**Cluster**: kube context `default` (k3s v1.36.3+k3s1). The operator identity is `system:masters`.
**Identity**: `system:serviceaccount:fathom-e2e:fathom-agent`.

**Goal**: the agent is admin in `fathom-dev` and `fathom-e2e`, and read-only everywhere else. It
cannot read Secrets and cannot write cluster-scoped objects.

Canonical manifests: [`k8s/prod-scoped-agent/rbac.yaml`](../../../k8s/prod-scoped-agent/rbac.yaml).

The reusable, idempotent, opt-in spec is `ui/tests/live/rbac-prod-scoped-setup.spec.ts`. It
runs only with `FATHOM_PROD_RBAC_SETUP=1`, through `ui/playwright.prod-rbac.config.ts`
(`pnpm -C ui e2e:prod-rbac`). The regular live suite ignores it.

## Pre-existing (not touched)

- [x] Namespace `fathom-e2e` and ServiceAccount `fathom-e2e/fathom-agent` already existed. They
  were not recreated or modified. The SA carries no `managed-by` label, and it was left as is.
- [x] `fathom-dev` exists and carries the label `fathom.dev/sandbox=true`.

## Objects created

Every object carries the label `app.kubernetes.io/managed-by=fathom-rbac-setup`.

| Object | Scope | roleRef | Method | Created (UTC) |
|---|---|---|---|---|
| ClusterRole `fathom-agent-cluster-read` | cluster | – | **kubectl fallback** (011 gap, see below) | 14:38:57 |
| ClusterRoleBinding `fathom-agent-view` | cluster | ClusterRole `view` | Fathom UI (create form, diff, confirm) | 14:52:17 |
| ClusterRoleBinding `fathom-agent-cluster-read` | cluster | ClusterRole `fathom-agent-cluster-read` | Fathom UI | 14:52:32 |
| RoleBinding `fathom-agent-admin` | `fathom-dev` | ClusterRole `admin` | Fathom UI | 14:52:47 |
| RoleBinding `fathom-agent-admin` | `fathom-e2e` | ClusterRole `admin` | Fathom UI (second run, see flake below) | 14:56:02 |

`fathom-agent-cluster-read` grants only get/list/watch. It covers:

- `nodes`, `namespaces` and `persistentvolumes`.
- `storageclasses.storage.k8s.io`, `customresourcedefinitions.apiextensions.k8s.io` and
  `apiservices.apiregistration.k8s.io`.
- `clusterroles`, `clusterrolebindings`, `roles` and `rolebindings` in
  `rbac.authorization.k8s.io`.
- `nodes` and `pods` in `metrics.k8s.io` (v1beta1 is served on this cluster).

The spec asserts that the role's verb set is exactly {get, list, watch}.

**ClusterRole creation**: checked the context, ran a server-side dry run, then ran
`kubectl --context default apply` with that single object. Nothing else was applied.

### Write guard used for the UI run

The devbridge gained a strict mode, so the prod UI run could not write anything but the plan:

- `--allow-create KIND/[NS/]NAME` (repeatable) permits only `CreateResourceYaml` of those exact
  objects. Every other write is refused, including writes in the sandbox namespace.
  - The backend creates with `api.create`, which returns 409 if the object exists and never
    replaces it.
- `--read-only` refuses every write.

`global-setup-prod-rbac.ts` passes the four bindings as `--allow-create` in setup mode, and
`--read-only` in verify mode. Each UI test also asserts three things:

- The page sent exactly one write.
- That write was the `CreateResourceYaml` of the expected object.
- The bridge refused nothing, and nothing was written before the confirm dialog.

Diff previews:

- [diff-ClusterRoleBinding-fathom-agent-view.png](prod-scoped-agent/diff-ClusterRoleBinding-fathom-agent-view.png)
- [diff-RoleBinding-fathom-e2e.png](prod-scoped-agent/diff-RoleBinding-fathom-e2e.png)

## can-i matrix

All checks are read-only access reviews. The `--as` column ran as the operator impersonating the
SA. The kubeconfig column ran as the real SA identity, using
`kubectl --kubeconfig ~/.kube/fathom-agent.yaml`.

- The kubeconfig was used only via `--kubeconfig` / `KUBECONFIG`. Its contents were never read.
- `kubectl auth whoami` returned `system:serviceaccount:fathom-e2e:fathom-agent`.

| Check | Expected | `--as` | kubeconfig |
|---|---|---|---|
| create deployments -n fathom-dev | yes | yes | yes |
| create deployments -n fathom-e2e | yes | yes | yes |
| create deployments -n default | no | no | no |
| delete pods -n default | no | no | no |
| get secrets -n default | no | no | no |
| create deployments -n kube-system | no | no | no |
| delete pods -n kube-system | no | no | no |
| get secrets -n kube-system | no | no | no |
| list pods -A | yes | yes | yes |
| list nodes | yes | yes | yes |
| create clusterrolebindings | no | no | no |

The verify spec also asserts `create rolebindings` = yes in both fathom-* namespaces, as the agent.

## Namespace-access lookup (011 US3/US4)

Ran the Access Control → Namespace Access lookup for ServiceAccount `fathom-e2e/fathom-agent`,
as the operator, with the bridge in strict mode. Screenshot:
[namespace-access.png](prod-scoped-agent/namespace-access.png).

- [x] **Cluster-wide**: comes only via `ClusterRoleBinding/fathom-agent-view` and
  `ClusterRoleBinding/fathom-agent-cluster-read`. The verbs are exactly {get, list, watch}.
- [x] **Per-namespace**: exactly `fathom-dev` and `fathom-e2e`, via `fathom-agent-admin`.
- [x] The lookup sent no writes.

## RBAC-aware UI as the agent

Fathom (web/devbridge, `--read-only` bridge) was connected with `KUBECONFIG=~/.kube/fathom-agent.yaml`,
context `prod-fathom-agent`. Spec: `ui/tests/live/rbac-prod-scoped-verify.spec.ts`
(`FATHOM_PROD_RBAC_VERIFY=1`). Result: 2/2 passed.

The gated "New RoleBinding" / "New ClusterRoleBinding" launcher reported these `data-capability`
values:

| Scope | Expected | Observed | Screenshot |
|---|---|---|---|
| RoleBinding @ fathom-dev | allowed, enabled | allowed, enabled | [png](prod-scoped-agent/rolebindings-fathom-dev.png) |
| RoleBinding @ fathom-e2e | allowed, enabled | allowed, enabled | [png](prod-scoped-agent/rolebindings-fathom-e2e.png) |
| RoleBinding @ default | denied, disabled | denied, disabled | [png](prod-scoped-agent/rolebindings-default.png) |
| RoleBinding @ kube-system | denied, disabled | denied, disabled | [png](prod-scoped-agent/rolebindings-kube-system.png) |
| ClusterRoleBinding (cluster) | denied, disabled | denied, disabled | [png](prod-scoped-agent/clusterrolebindings.png) |

No write was sent.

## Bugs and gaps found

1. **Fixed: the binding create form had no labels field.** The `managed-by` label could not be
   set through the UI.
   - Added a create-mode `Labels` input (`rbac-binding-labels`) to `RoleBindingForm`. It is
     validated by `parseLabels`, which applies the k8s key/value syntax.
   - `buildRoleBindingManifest` now emits `metadata.labels`.
   - Unit tests are in `ui/tests/unit/rolebindingForm.test.ts`.
2. **Gap (not fixed, by 011 design): no Role/ClusterRole create flow.** 011 only reads roles, so
   the ClusterRole went through the kubectl fallback above. A follow-up would be a
   "New Role / ClusterRole" rules editor.
3. **Flake: "Loading Role Bindings" hang.** On the first run, the Role Bindings pane for
   `fathom-e2e` (then an empty namespace) stayed on "Loading Role Bindings" for more than 20s, so
   the launcher never rendered. An identical rerun loaded the pane immediately and created the
   binding. The flake was not reproduced; worth a look at how the list status handles an empty
   namespace-scoped list right after a namespace switch.
   - **Root cause (2026-09-26):** not the empty namespace. The UI and reply list keys match, and
     `refresh_role_bindings` always emits, even an empty list. The backend runs every command under
     one `BackendState` mutex (`src/backend.rs` `run_cmd`). The page's bootstrap `Initialize` holds
     that mutex through the metrics probe, Prometheus discovery, port-forward restore and
     `ensure_metrics_port_forward` (5s HTTP timeout) before it refreshes the core lists.
     `app.open()` returns once `ContextsUpdated` fills the combobox, so `SetNamespaceFilter` and
     `RefreshRoleBindings` queue behind a cold `Initialize` that can take over 20s. A warm rerun is
     fast.
   - **Worked around in the spec first:** `createThroughUi` waited up to 90s for the launcher.
   - **Fixed in the backend (2026-09-26):** `Initialize` and `SetContext` now do only the required
     work (contexts, client, core lists) under the lock. Metrics-API probing, Prometheus
     discovery, saved port-forward restore and the Prometheus auto-forward run in `post_connect`
     (`src/backend/cmd/cluster.rs`) after `run_cmd` releases the lock, re-taking it only briefly
     to record results and dropping them if the context changed meanwhile. The spec is back on
     the default expect timeout. Guarded by the Rust test
     `refresh_is_not_blocked_behind_slow_post_connect_discovery`.
   - **Still open (product):** `Initialize` should not hold the command lock through optional
     integration work. Run the core lists first and move the integrations to a follow-up
     command.
4. **Minor: noisy 403 toast for a restricted identity.** As the agent, the status bar shows
   "Background Hubble traffic poll failed … 403 Forbidden". The background pollers should treat
   403 as "not permitted" and stay quiet.
5. **Test infrastructure changes:**
   - The nav labels are "Role Bindings" and "Cluster Role Bindings", with spaces.
   - Added `data-testid="rbac-namespace-access-per-namespace"`.
   - The devbridge gained the strict `--allow-create` and `--read-only` modes.

## Reproduce

```sh
# 1. ClusterRole (no UI flow in 011): create only that object from the manifest
kubectl --context <ctx> create -f <(yq 'select(.kind == "ClusterRole")' k8s/prod-scoped-agent/rbac.yaml)
# 2. Bindings through the UI (idempotent: existing objects are verified, never touched)
FATHOM_PROD_RBAC_SETUP=1 FATHOM_LIVE_CONTEXT=<ctx> pnpm -C ui e2e:prod-rbac
# 3. Read-only checks as the agent (operator-minted kubeconfig)
FATHOM_PROD_RBAC_VERIFY=1 KUBECONFIG=~/.kube/fathom-agent.yaml \
  FATHOM_LIVE_CONTEXT=prod-fathom-agent pnpm -C ui e2e:prod-rbac
```
