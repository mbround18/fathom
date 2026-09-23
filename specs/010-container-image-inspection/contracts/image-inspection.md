# Contract: Container Image Inspection

## Command

`InspectContainerImage { namespace: string; pod: string; container: string }`

Sent by the UI only when the operator opens a container's "Image" tab (never eagerly). The
backend:
1. Re-reads the live Pod (`Api<Pod>::get`) to resolve `spec.containers[container].image` and
   `spec.imagePullSecrets`.
2. Resolves the image reference (default registry/tag conventions applied if the reference omits
   them).
3. Attempts an anonymous manifest fetch (`oci-client`). On an auth challenge, retries per-secret
   from `imagePullSecrets` (parsing `.dockerconfigjson`).
4. Resolves a manifest list to the platform-matching entry, if applicable.
5. Fetches the image config blob (never layer blob bodies).
6. Emits exactly one `ContainerImageInspected` (success) or `ContainerImageInspectFailed` (error)
   event, routed like other Pod detail reads.
7. On success, caches the result in memory by resolved digest; a later `InspectContainerImage`
   for a different container sharing that digest reuses the cache with no registry round-trip.

## Event: success

```json
{
  "type": "ContainerImageInspected",
  "data": {
    "namespace": "default", "pod": "web-abc123", "container": "app",
    "inspection": {
      "image_ref": { "registry": "docker.io", "repository": "library/nginx", "tag": "1.27", "digest": "sha256:..." },
      "total_size_bytes": 45231000,
      "layers": [ { "digest": "sha256:...", "size_bytes": 3200000, "media_type": "application/vnd.oci.image.layer.v1.tar+gzip", "command": "COPY file:... in /", "empty_layer": false } ],
      "created": "2026-08-01T12:00:00Z",
      "base_image": { "label": "alpine:3.19 (best-effort)", "confidence": "best-effort" }
    }
  }
}
```

## Event: error

```json
{
  "type": "ContainerImageInspectFailed",
  "data": {
    "namespace": "default", "pod": "web-abc123", "container": "app",
    "error": { "kind": "AuthenticationFailed", "detail": "no imagePullSecrets grant access to ghcr.io" }
  }
}
```

Error kinds: `RegistryUnreachable`, `ImageNotFound`, `AuthenticationFailed`,
`AuthMethodUnsupported`, `Other`. The UI renders one distinguishable message per kind (FR-009);
never a blank/spinner-forever state.

## Invariants (tested)

- No `InspectContainerImage` command is ever sent before the operator opens that container's
  "Image" tab (frontend: no effect fires on mount of the Details tab; only on Image tab mount/select).
- No layer blob bodies are fetched — asserted at the backend unit-test level (mock registry server
  counts blob-body requests are 0) and documented as an integration invariant.
- Read-only: this feature issues no `PATCH`/`PUT`/`DELETE` against any Kubernetes object or
  registry endpoint.
