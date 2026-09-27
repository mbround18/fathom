# Data Model: Role and ClusterRole create/edit

## UI (ui/src/components/rbac/roleYaml.ts)

| Entity | Fields | Notes |
|---|---|---|
| `RoleKind` | `'Role' \| 'ClusterRole'` | |
| `RuleInput` | `apiGroups: string[]`, `resources: string[]`, `verbs: string[]`, `resourceNames: string[]`, `nonResourceURLs: string[]` | Core group is `''`. `nonResourceURLs` only valid on a ClusterRole. |
| `PresetId` | `'read-only' \| 'edit' \| 'custom'` | read-only = `get list watch`; edit = the 8 standard verbs. |
| `STANDARD_VERBS` | `get list watch create update patch delete deletecollection` | Checkbox order; extra verbs follow. |

Manifest emitted by `buildRoleManifest`:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role            # or ClusterRole
metadata:
  name: <name>
  namespace: <ns>     # Role only
rules:
  - apiGroups: [""]
    resources: [pods]
    resourceNames: [...]   # only when set
    verbs: [get, list, watch]
  - nonResourceURLs: [/healthz]   # ClusterRole only; no apiGroups/resources
    verbs: [get]
```

Edit mode does not use `buildRoleManifest`: `patchRoleRules(rawJson, kind, rules)` replaces `rules` in the live object and keeps everything else.

## Protocol (src/protocol/v1/roles.rs)

| Type | Fields |
|---|---|
| `ApiResourceName` | `name: String`, `namespaced: bool`, `verbs: Vec<String>` |
| `ApiGroupResources` | `group: String` (`""` = core), `resources: Vec<ApiResourceName>` |
| `ApiResourceCatalog` | `groups: Vec<ApiGroupResources>`, `failed_groups: Vec<String>` |

## Store

`rbacAccess.apiResources: ApiResourceCatalog | undefined`, replaced on every `ApiResourcesUpdated`.
