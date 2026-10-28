# Contract: registry port and requests (062)

## Port (`depths-core`, agnostic, no I/O)

The adapter is async (`oci-client` is). The port returns boxed futures, so it can be held as
`Arc<dyn Registry>` without adding `async-trait` to normal dependencies:

```rust
pub type RegistryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RegistryError>> + Send + 'a>>;

pub trait Registry: Send + Sync {
    /// HEAD the tag. Never GETs. `Ok(None)` when the registry sent no digest header.
    fn head_digest<'a>(&'a self, r: &'a ImageRef, auth: &'a Credentials) -> RegistryFuture<'a, Option<Digest>>;
    /// GET a manifest or index by digest (counted as a pull by Docker Hub).
    fn manifest<'a>(&'a self, r: &'a ImageRef, digest: &'a Digest, auth: &'a Credentials) -> RegistryFuture<'a, Manifest>;
    /// GET a config blob by digest (not counted).
    fn config<'a>(&'a self, r: &'a ImageRef, digest: &'a Digest, auth: &'a Credentials) -> RegistryFuture<'a, ImageConfig>;
}
pub enum Manifest { Index(Vec<IndexEntry>, Annotations), Image { config: Digest, annotations: Annotations } }
pub struct IndexEntry { pub digest: Digest, pub platform: Platform }
pub struct Platform { pub os: String, pub architecture: String, pub variant: Option<String> }
pub struct ImageConfig { pub user: Option<String>, pub labels: BTreeMap<String, String>, pub created: Option<String> }
pub enum Credentials { Anonymous, Basic { user: String, pass: SecretString }, IdentityToken(SecretString) }
pub enum RegistryError {
    Unauthorized,                    // 401/403 after credentials
    NotFound,                        // tag or repo gone
    RateLimited { retry_after: Option<Duration> },  // 429
    Unreachable(String),             // DNS, TLS, connect, 5xx
    Protocol(String),                // unexpected media type, bad JSON
}
```

`Credentials` has a redacting `Debug`. The resolution and comparison logic (research R1) is a pure
function over this port and lives in `depths-core::updates::check`; the fake registry in tests
implements the port and counts calls per method.

## Adapter (`crates/depths-registry-oci`)

- `oci-client` 0.18 (owner approval: plan.md "Dependencies"), `rustls-tls`, no `test-registry`
  feature in normal builds.
- `head_digest` → `Client::fetch_manifest_digest`; the client is built with the four accepted
  manifest media types. See research R1 for its GET fallback when a registry omits the header.
- `manifest` → `Client::pull_manifest_raw` by digest, then parsed with `oci-spec`'s index/manifest
  types (Docker list and v2 media types accepted).
- `config` → `Client::pull_blob` into a 1 MiB-capped buffer. Larger configs are `Protocol`.
- Anonymous first; on `Unauthorized` with credentials available, retry once with them (as Fathom's
  `images.rs` does).
- Timeouts: 10 s connect, 30 s per request. No retries inside the adapter; backoff is the
  scheduler's (research R5).
- Platform choice: the host's `os/architecture/variant` from `docker info`
  (`OSType`, `Architecture` normalised: `x86_64`→`amd64`, `aarch64`→`arm64` + variant `v8`), or the
  service's `platform:`.

## Credentials file (`$DOCKER_CONFIG/config.json`)

Read per check, never cached across checks, never written.

| Field | Used |
|---|---|
| `auths.<key>.auth` | base64 `user:password` → `Basic` |
| `auths.<key>.username` + `password` | `Basic` |
| `auths.<key>.identitytoken` | `IdentityToken` (OAuth refresh token in the token request) |
| `credsStore`, `credHelpers.<host>` | Not supported: service shows `check_failed` "credential helper `<name>` not available". |
| anything else | ignored |

Key matching: exact host, `https://host`, `https://host/v1/`, `http://host`. Docker Hub
(`docker.io`) matches `https://index.docker.io/v1/`, `index.docker.io`, `docker.io`,
`registry-1.docker.io`.

A missing file means anonymous. An unreadable file: every private check fails with "docker config
unreadable: <path>"; public images still check anonymously. The file's contents never appear in
logs, spans, audit entries, the protocol or Depths' state.

## Request budget (tested, SC-007)

With the fake registry and a fake clock, 30 stacks × one service each, 6 h interval, 7 days:

| Scenario | HEAD | manifest GET | blob GET |
|---|---|---|---|
| No tag moves | 30 × 28 = 840 | 0 | 0 |
| One tag moves once, multi-arch | 840 | 2 (index + platform), once | 1 |
| Same tag moves, 10 stacks share the image | 840 | 2, once | 1 |
| Running side under containerd store needs its own platform manifest | 840 | +1 per old index, once | 0 |
| Registry 429 for a day | fewer (backoff) | 0 | 0 |

The integration test (`DEPTHS_IT_DOCKER=1`) repeats the first two rows against a local
`distribution` registry and reads its access log.
