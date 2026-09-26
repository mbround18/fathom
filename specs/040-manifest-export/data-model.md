# Data Model: 040 Manifest Export

## Protocol types (Rust, `src/protocol/v1/manifest_export.rs`, TS generated)

### ManifestObjectRef
| Field | Type | Notes |
|---|---|---|
| api_version | String | e.g. `apps/v1`, `v1`, `example.com/v1alpha1` |
| kind | String | e.g. `Deployment` |
| namespace | Option<String> | `None` for cluster-scoped |
| name | String | |

### FetchedManifest
| Field | Type | Notes |
|---|---|---|
| index | u32 | position in the request's `objects` |
| raw_json | String | the object as the API server returned it (Secrets redacted per FR-011) |

### ManifestFetchError
| Field | Type | Notes |
|---|---|---|
| index | u32 | |
| message | String | the API server's or discovery's error text, verbatim |

### KubeconfigCredentialKind (adjacently tagged enum)
`Exec { command: String }`, `AuthProvider { name: String }`, `Token`, `TokenFile`, `ClientCertificate`, `Basic`, `None`

### KubeconfigExportPreview
| Field | Type | Notes |
|---|---|---|
| context | String | |
| redacted_yaml | String | every secret field is `<redacted>` |
| credential | KubeconfigCredentialKind | |
| embedded_secret_fields | Vec<String> | e.g. `["token"]`, `["auth-provider.config.id-token"]` |
| file_references | Vec<String> | e.g. `["client-key: /home/u/.minikube/client.key"]` |
| warnings | Vec<String> | |
| suggested_path | String | `~/.kube/fathom-<context>.yaml`, expanded |
| source_path | String | the kubeconfig file read (display only) |

### Validation rules
- `FetchManifests.objects.len() <= 500`. Otherwise the whole request is answered with one error and nothing is fetched.
- `ExportKubeconfig.path` must be absolute after `~` expansion, its parent must exist, and it must not equal the source kubeconfig. It must not exist unless `overwrite`.

## UI state (`ui/src/features/manifest-export/state.ts`, store slice `manifestExport`)

```ts
interface ManifestExportState {
  fetches: Record<string /*request_id*/, { manifests: FetchedManifest[]; errors: ManifestFetchError[] }>
  kubeconfigPreview: { seq: number; data?: KubeconfigExportPreview; error?: string } | null
  kubeconfigResult: { seq: number; ok: boolean; path?: string; embedded: boolean; message?: string } | null
}
```

The export dialog's open state lives in a separate small zustand store (`useExportDialog`), so the main store stays untouched except for the reply slice.

## Rendered document (UI only)
`ExportDocument { ref: ManifestObjectRef; yaml: string; warnings: string[] }`. It is derived from raw JSON plus the mode by `renderManifest(raw, {clean})`. A bundle is `docs.map(d => d.yaml).join('\n---\n')`, and always ends with a newline.
