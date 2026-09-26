# Protocol contract: 040 Manifest Export

All additions are additive variants on `BackendCmdV1` and `SystemEvent` (protocol v1, non-breaking). Replies go to `Route::global(system)`.

## Commands

### `FetchManifests` (ReadOnly, hosted: Allowed)
```json
{ "type": "FetchManifests", "data": { "request_id": "exp-1", "objects": [
  { "api_version": "apps/v1", "kind": "Deployment", "namespace": "default", "name": "web" } ] } }
```
Reply: exactly one `ManifestsFetched`.

### `PreviewKubeconfigExport` (ReadOnly, hosted: Denied(Contexts))
```json
{ "type": "PreviewKubeconfigExport", "data": { "context": "kind-fathom-dev" } }
```
Reply: `KubeconfigExportPreviewed` (ok) or `KubeconfigExportFinished { ok: false, ... }` on error (unknown context, unreadable kubeconfig, hosted).

### `ExportKubeconfig` (Local, hosted: Denied(Contexts))
```json
{ "type": "ExportKubeconfig", "data": { "context": "kind-fathom-dev",
  "path": "~/.kube/fathom-kind-fathom-dev.yaml", "include_embedded_credentials": false, "overwrite": false } }
```
Sent only after the UI confirmation dialog. Reply: `KubeconfigExportFinished`.

## Events (`SystemEvent`)

### `ManifestsFetched`
```json
{ "type": "ManifestsFetched", "data": { "request_id": "exp-1",
  "manifests": [ { "index": 0, "raw_json": "{...}" } ],
  "errors": [ { "index": 1, "message": "deployments.apps \"gone\" not found" } ] } }
```
Invariant: every requested index appears in exactly one of `manifests` or `errors`. The only exception is a request over the cap, which yields `errors: [{index: 0, message: "too many objects ..."}]` and no manifests.

### `KubeconfigExportPreviewed`
`data`: `KubeconfigExportPreview` (see data-model.md). Invariant: `redacted_yaml` contains no value of any embedded secret field.

### `KubeconfigExportFinished`
```json
{ "type": "KubeconfigExportFinished", "data": { "context": "c", "ok": true,
  "path": "/home/u/.kube/fathom-c.yaml", "embedded_credentials_written": false, "message": null } }
```
On failure `ok: false`, `path` is null, and `message` carries the error text, which never includes credential values.

## Security invariants (tested)
1. No credential value appears in any event for any kubeconfig fixture, whether exec, token, client-key-data, auth-provider id-token or basic auth.
2. The written file has mode `0600` on Unix, whether freshly created or overwritten.
3. The source kubeconfig path is refused as a target.
4. Hosted access refuses both kubeconfig commands (policy and backend).
5. `tracing` calls in `kubeconfig_export.rs` reference only the context and path.
