# Protocol contract: 042-role-create

## New command

`{ "type": "RefreshApiResources" }`

- Mutation class: `ReadOnly`. Section: `access/roles`. Not audited.
- Requires a connected cluster. Reply: one `ApiResourcesUpdated` event. Errors go through the usual command error path (`CommandFinished { ok: false, error }`).

## New event (ResourceReadEvent)

```json
{
  "type": "ApiResourcesUpdated",
  "data": {
    "groups": [
      { "group": "", "resources": [ { "name": "pods", "namespaced": true, "verbs": ["get", "list"] },
                                    { "name": "pods/log", "namespaced": true, "verbs": ["get"] } ] },
      { "group": "apps", "resources": [ ... ] }
    ],
    "failed_groups": ["metrics.k8s.io"]
  }
}
```

- One entry per group at its preferred version. Subresources are listed as `<resource>/<sub>`.
- A group whose resource list could not be fetched is named in `failed_groups` and omitted from `groups`.

## Reused commands (unchanged)

| Action | Command | Notes |
|---|---|---|
| Create | `CreateResourceYaml { target, yaml }` | Audited (026). |
| Edit | `ApplyResourceYaml { target, yaml, expected_resource_version }` | Audited; conflict on a stale version. |
| After a write | `RefreshRoles` / `RefreshClusterRoles` | RBAC lists are not watched. |
| After an edit | `SelectRole { namespace, name }` / `SelectClusterRole { name }` | Reloads the inspector. |

## Devbridge guard

Cluster-scoped writes stay blocked except `ClusterRole` create, apply or delete with a name starting `fathom-e2e-` (and longer than the prefix), never in strict mode. `ClusterRoleBinding` stays blocked.
