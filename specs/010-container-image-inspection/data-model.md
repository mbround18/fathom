# Data Model: Container Image Inspection

## Protocol types (`src/protocol/v1/images.rs`, new)

```rust
pub struct ImageRef {
    pub registry: String,       // resolved host, e.g. "docker.io", "ghcr.io"
    pub repository: String,     // e.g. "library/nginx"
    pub tag: Option<String>,
    pub digest: String,         // resolved manifest digest (always present once fetched)
}

pub struct ImageLayer {
    pub digest: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub command: Option<String>, // matched from config.history, best-effort by order
    pub empty_layer: bool,       // config.history entries with `empty_layer: true` have no blob
}

pub struct BaseImageGuess {
    pub label: String,          // e.g. "alpine:3.19 (best-effort)"
    pub confidence: String,     // "best-effort" (only value today; room to grow)
}

pub struct ImageInspection {
    pub image_ref: ImageRef,
    pub total_size_bytes: u64,
    pub layers: Vec<ImageLayer>,
    pub created: Option<String>,     // RFC3339, from config.created
    pub base_image: Option<BaseImageGuess>,
}

pub enum ImageInspectError {
    RegistryUnreachable { detail: String },
    ImageNotFound,
    AuthenticationFailed { detail: String },
    AuthMethodUnsupported { detail: String },
    Other { detail: String },
}
```

## Commands / Events (`src/protocol/v1/commands.rs`, `events.rs`)

- `BackendCmd::InspectContainerImage { namespace: String, pod: String, container: String }`
  — new command, dispatched when the operator opens a container's "Image" tab.
- `ResourceReadEvent::ContainerImageInspected { namespace, pod, container, result: Result<ImageInspection, ImageInspectError> }`
  (serialized as `{ Ok: ... } | { Err: ... }` via serde, matching existing Result-shaped events
  in the protocol if that convention exists — otherwise two explicit variants: `ContainerImageInspected(ImageInspection)` and
  `ContainerImageInspectFailed(ImageInspectError)`, whichever matches repo convention found in
  src/protocol/v1/events.rs during implementation).

Routing: reuses `Route::namespaced(section_keys::workloads_pods(), ctx, Some(namespace))`, same
as `PodDetailsUpdated`, keyed additionally by `(pod, container)` in the frontend store so multiple
containers' results don't collide.

## Backend state (`src/backend/state.rs` or new `src/backend/images.rs`)

- `image_inspect_cache: HashMap<String /* digest */, ImageInspection>` on `BackendState`,
  in-memory only, cleared on cluster/context switch (mirrors other per-session caches).
- Cache lookup path: resolve `ImageRef` (registry+repo+tag) → anonymous HEAD/GET manifest → get
  digest → check cache by digest before fetching config/layers.

## Frontend

- `ui/src/store` gains a small keyed cache/slice: `imageInspections: Record<string, ImageInspectState>`
  keyed by `${namespace}/${pod}/${container}`, each `{ status: 'idle'|'loading'|'ready'|'error', data?, error? }`.
- `ui/src/components/panels/ImageInspectPanel.tsx` (new): container picker (when >1 container) +
  fetch-on-select button/effect + rendered manifest/layers/history/base-image, all testids
  prefixed `image-inspect-`.
- Wired into `Inspector.tsx` as a new `image` tab shown only for `target.kind === 'Pod'`.
