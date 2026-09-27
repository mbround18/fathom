# Live validation: 042-role-create (kind `fathom-dev`)

Run on 2026-09-26, `tests/live/rbac-role-create.spec.ts`, project `chromium-wide`: 2 passed.

- [x] Discovery feeds the resource picker (namespaced only for a Role, cluster-scoped too for a ClusterRole)
- [x] Role `fathom-e2e-role-*` created in `fathom-dev` through the UI; rules verified with kubectl
- [x] Row appears in the Roles list after create (post-apply refresh)
- [x] Edit from the inspector adds `create`; verified with kubectl
- [x] Delete from the list (bulk delete, confirm); kubectl confirms it is gone
- [x] ClusterRole `fathom-e2e-cr-*` created, edited (`patch` added) and deleted the same way; bind shortcut offered
- [x] No `fathom-e2e-*` Roles or ClusterRoles left in the cluster afterwards
- [x] Only `kind-fathom-dev` used, through a temporary kubeconfig
