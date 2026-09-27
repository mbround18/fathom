# Research: Role and ClusterRole create/edit

## Discovery source

- **Decision**: A dedicated `RefreshApiResources` walk of `/api` and `/apis`, preferred version per group, subresources included.
- **Rationale**: The existing CRD and resource lists do not carry subresources (`pods/log`, `deployments/scale`) or cluster-scope flags for built-ins, and RBAC rules name plurals exactly as discovery does.
- **Alternatives**: `kube::discovery::Discovery::run`, rejected because one unavailable aggregated API (`metrics.k8s.io` without metrics-server) fails the whole run.

## Write path

- **Decision**: Reuse the YAML edit session (`openCreateEditSession` / `openEditSession`).
- **Rationale**: It already provides confirm, diff, conflict detection, audit recording and the success/error events. A new RBAC-specific command would duplicate all of it.

## Escalation detection

- **Decision**: Match the API server message `attempting to grant RBAC permissions not currently held` and extract the `{APIGroups:...}` / `{NonResourceURLs:...}` entries it lists.
- **Rationale**: The message is stable across supported Kubernetes versions and is the only place the missing permissions are listed. A pre-flight SSAR per rule would need one review per (group, resource, verb) tuple and still miss `resourceNames` subtleties.

## Access check

- **Decision**: `useMutationAccess` with verb `create` (create) or `update` (edit) on `roles`/`clusterroles` in the target scope.
- **Rationale**: Same hook as the RoleBinding form (018). Escalation cannot be predicted by it, hence the explicit error handling.

## Live ClusterRole writes

- **Decision**: The devbridge guard allows only `fathom-e2e-*` ClusterRoles, never in strict mode. The test helper `e2eClusterRole` refuses any other name.
- **Rationale**: Cluster-scoped writes are otherwise blocked in live tests. A name prefix fence is simple to audit and cannot reach real roles.
