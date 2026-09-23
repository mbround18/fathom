# Quickstart Validation: RBAC / OIDC Administration

Prerequisites: a kubeconfig context Fathom can connect to (any cluster; a kind/minikube cluster
is sufficient — no live OIDC issuer required, since Story 5 never talks to the cluster).

## Story 1 — Browse Roles/ClusterRoles
1. `cargo build && pnpm -C ui dev` (or run the Tauri app).
2. Open Access Control > Roles. Expect a list grouped/filterable by namespace, each row
   selectable to show resolved rules (API groups/resources/verbs) in a table, not raw YAML.
3. Switch to ClusterRoles; same but no namespace grouping.
4. If your identity lacks `list` on `roles` in some namespace, expect that namespace named
   explicitly as inaccessible with the backend error shown, not silently dropped.

## Story 2 — Create a RoleBinding
1. Open Access Control > RoleBindings > New.
2. Pick an existing Role, enter a namespace, add a subject (Group `test-group`).
3. Click review: expect a `YamlDiffView` showing the full proposed object (nothing removed,
   everything added, since it's a create).
4. Confirm via `ConfirmDialog`: expect the binding to appear in the RoleBindings list only after
   the apply succeeds (watch network/event order — no optimistic insert).
5. Cancel instead on a second attempt: expect no object created (`kubectl get rolebinding` count
   unchanged).

## Story 3 — Edit/delete a RoleBinding
1. Open the binding created above, add a second subject, review diff, confirm — expect it
   updated only after backend confirms.
2. In a second terminal, `kubectl delete rolebinding <name> -n <ns>` while the edit view is still
   open with a stale local copy; attempt to apply the pending edit; expect a conflict/not-found
   message and a reload affordance, not a silent failure or overwrite.
3. Delete a binding via the UI; expect the confirm dialog to name kind+namespace+name exactly,
   and the row to disappear only post-confirmation from the backend.

## Story 4 — Namespace-access lookup
1. Open Access Control > Namespace Access, enter `test-group` (the Group subject used above).
2. Expect the namespace from Story 2/3 listed with the resolved verb/resource set.
3. Enter a subject with a `ClusterRoleBinding` (e.g. bind `test-group` to `view` cluster-wide via
   `kubectl create clusterrolebinding`); expect it marked cluster-wide, not enumerated per
   namespace.
4. Enter a nonsense subject name; expect an explicit "no bindings found" state.

## Story 5 — OIDC config generator
1. Open Access Control > OIDC Configuration.
2. Expect an upfront statement that Fathom cannot apply API-server flag changes itself, plus a
   best-effort kubeadm/managed-cluster indicator.
3. Fill in issuer URL, client ID, audience, username claim, groups claim; generate.
4. Expect a visibly distinct "apply this yourself" panel (different styling from the
   diff-preview used in Stories 2/3) with copyable `kube-apiserver` flags and provider-labeled
   instructions, and an export action. Confirm no network/cluster call is made when generating
   (check devtools network/backend command log — no command should fire besides the optional
   kubeadm-detection read on page load).

## Automated coverage entry points
- `cargo test --test protocol_serde` — round-trip coverage for all new types.
- `cargo test -p fathom --lib rbac_access` (or equivalent unit test module path) — namespace
  access resolution + self-access mapping against fixtures.
- `pnpm -C ui test` — component-level Vitest coverage (dangling-roleRef badge, OIDC flag
  generation, form validation).
- `pnpm -C ui exec playwright test us1-rbac- us2-rbac- us3-rbac- us4-rbac- us5-rbac-` — e2e per
  user story against the scripted fake backend.
