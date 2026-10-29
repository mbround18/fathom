# Contract: `crates/oci-registry` and the registry requests (spec 061, shared with 062)

`crates/oci-registry` is a HEAD-first OCI distribution client, shared by Keel (this spec) and
Depths (spec 062). It is an adapter: it holds no Keel or Depths types, and each app maps its
results into its own model. Whichever spec is implemented first creates the crate; the other uses
it as it is and extends it only through this contract.

This contract has three parts: the crate's API, the exact set of HTTP requests it may make, and the
fake registry its tests (and the apps' tests) use. Anything not listed here is a bug. The crate's
tests assert the request list against the fake registry's request log.

## Crate

`crates/oci-registry/Cargo.toml` (picked up by the workspace's `crates/*` glob):

```toml
[package]
name = "oci-registry"
version = "0.1.0"
description = "HEAD-first OCI distribution client shared by the suite's servers"
edition = "2024"
license = "Apache-2.0"
publish = false

[features]
# The in-process fake registry and the base-URL override. Tests only; no application enables it
# outside its dev-dependencies.
test-util = ["dep:axum", "tokio/net"]

[dependencies]
reqwest = { version = "0.13.5", features = ["json", "default-tls"] }   # as web-auth
secrecy = "0.10"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
sha2 = "0.11"
thiserror = "2"
tokio = { version = "1.54.1", features = ["time"] }
tracing = "0.1.44"
axum = { version = "0.8", optional = true }

[dev-dependencies]
oci-registry = { path = ".", features = ["test-util"] }
tokio = { version = "1.54.1", features = ["full", "test-util"] }
```

The workspace has no `[workspace.dependencies]` table, so versions are declared here, matching
the ones sibling crates already use. Nothing new enters the lockfile: reqwest is already compiled
into keel-server and depths-server through `web-auth`.

## API

```rust
/// `sha256:` followed by 64 lowercase hex characters. Parse-validated.
pub struct Digest(String);
impl Digest { pub fn parse(s: &str) -> Result<Digest, RegistryError>; pub fn as_str(&self) -> &str; }

pub enum Reference { Tag(String), Digest(Digest) }

pub struct ImageRef { /* registry host, repository, reference */ }
impl ImageRef {
    /// `[registry/]repository[:tag|@digest]`. Docker Hub defaults: no registry, `docker.io` or
    /// `index.docker.io` mean `registry-1.docker.io`; a single-segment repository on Docker Hub
    /// gets `library/`; no tag means `latest`.
    pub fn parse(s: &str) -> Result<ImageRef, RegistryError>;
    pub fn registry(&self) -> &str;          // API host[:port], e.g. "registry-1.docker.io"
    pub fn repository(&self) -> &str;        // e.g. "mbround18/keel", "library/nginx"
    pub fn reference(&self) -> &Reference;
    pub fn with_digest(&self, d: Digest) -> ImageRef;   // same registry and repository
}
impl std::fmt::Display for ImageRef { /* never includes credentials */ }

pub enum Credentials {
    Anonymous,
    Basic { username: String, password: secrecy::SecretString },
}
// `Debug` prints `Basic { username: "…", password: "[redacted]" }`. No log line, span field or
// error message ever contains the password or a token.

pub struct Platform { pub os: String, pub architecture: String, pub variant: Option<String> }
impl Platform {
    /// From `std::env::consts`: `x86_64` → `amd64`, `aarch64` → `arm64` (variant `v8`), os `linux`.
    pub fn current() -> Platform;
}

#[derive(Default, Clone, Copy)]
pub struct RequestCounts {
    pub head: u32,          // every HEAD
    pub get: u32,           // every GET: token, manifest and blob
    pub manifest_get: u32,  // the GETs on /v2/*/manifests/*, which registries count as pulls
}

/// Every call returns its result together with the requests it made, success or failure.
pub struct Counted<T> { pub result: Result<T, RegistryError>, pub requests: RequestCounts }

pub struct Resolved {
    pub index_digest: Option<Digest>,   // set when the digest named an index or manifest list
    pub manifest_digest: Digest,        // the platform manifest
    pub config_digest: Digest,
    pub labels: BTreeMap<String, String>,   // config.Labels (empty when absent)
    pub user: Option<String>,               // config.User
    pub created: Option<String>,            // the config's top-level `created`, as given (RFC 3339)
}

pub struct Client { /* reqwest clients, token cache */ }
impl Client {
    /// `https` only. `user_agent` is the caller's, e.g. `keel/0.2.0 (update-check)`.
    pub fn new(user_agent: &str) -> Result<Client, RegistryError>;
    /// Feature `test-util` only: every registry and realm request goes to `base` (plain `http`
    /// allowed), so tests point the client at the fake registry.
    #[cfg(feature = "test-util")]
    pub fn with_base_url(user_agent: &str, base: reqwest::Url) -> Client;

    /// HEAD the manifest. Never falls back to GET.
    pub async fn head_digest(&self, r: &ImageRef, c: &Credentials) -> Counted<Digest>;
    /// `r` must be by digest (`NotByDigest` otherwise, with no request made).
    pub async fn resolve(&self, r: &ImageRef, p: &Platform, c: &Credentials) -> Counted<Resolved>;
}

#[derive(thiserror::Error)]
pub enum RegistryError {
    InvalidReference(String),   // code "invalid_reference"
    NotByDigest,                // "not_by_digest"
    AuthFailed,                 // "auth_failed"
    NotFound,                   // "not_found"
    // "rate_limited". `retry_after` comes from the 429's `Retry-After` header: delta-seconds, or
    // an IMF-fixdate HTTP-date (a date in the past is zero). Absent or unparsable is `None`.
    // Depths' backoff honours it; Keel ignores it.
    RateLimited { retry_after: Option<std::time::Duration> },
    Timeout,                    // "timeout"
    Network,                    // "network"
    BadResponse(&'static str),  // "bad_response"; the str is a short reason for logs
    NoDigestHeader,             // "no_digest_header"
    NoImageForPlatform,         // "no_image_for_platform"
}
impl RegistryError { pub fn code(&self) -> &'static str; }
```

Module layout: `src/lib.rs` (re-exports), `src/reference.rs` (`ImageRef`, `Digest`,
`Reference`), `src/auth.rs` (`Credentials`, challenge parsing, token cache), `src/client.rs`
(`Client`, `head_digest`, `resolve`, `RequestCounts`), `src/manifest.rs` (index, manifest and
config parsing, platform selection), `src/error.rs`, `src/testing.rs` (feature `test-util`).

## Reference resolution

| Reference | Registry API base | Repository | Reference |
|---|---|---|---|
| `mbround18/keel:latest` | `https://registry-1.docker.io` | `mbround18/keel` | tag `latest` |
| `nginx` | `https://registry-1.docker.io` | `library/nginx` | tag `latest` |
| `docker.io/library/foo` | `https://registry-1.docker.io` | `library/foo` | tag `latest` |
| `index.docker.io/mbround18/keel@sha256:<64 hex>` | `https://registry-1.docker.io` | `mbround18/keel` | digest |
| `ghcr.io/org/keel:edge` | `https://ghcr.io` | `org/keel` | tag `edge` |
| `registry.example:5000/keel` | `https://registry.example:5000` | `keel` | tag `latest` |

`docker.io` and `index.docker.io` map to `registry-1.docker.io`. Nothing else is aliased, and
`library/` is added only on Docker Hub. The first path segment is a registry only when it contains
a `.` or a `:`, or is `localhost`. Upper-case repositories and tags over 128 characters are
`InvalidReference`.

## Requests

`Accept` for every manifest request, HEAD or GET, sent exactly as:

```text
application/vnd.oci.image.index.v1+json,
application/vnd.docker.distribution.manifest.list.v2+json,
application/vnd.oci.image.manifest.v1+json,
application/vnd.docker.distribution.manifest.v2+json
```

`User-Agent`: the caller's string. Keel sends `keel/<version or revision> (update-check)`.

| # | Request | Made by | When | Counted in | Counts as a pull on Docker Hub |
|---|---|---|---|---|---|
| 1 | `HEAD /v2/<repo>/manifests/<tag or digest>` without a token | `head_digest` | No valid cached token for this scope | `head` | No |
| 2 | `GET <realm>?service=<service>&scope=repository:<repo>:pull` | either | After a 401 carrying `Bearer` | `get` | No |
| 3 | `HEAD /v2/<repo>/manifests/<tag or digest>` with `Authorization: Bearer <token>` | `head_digest` | After #2, or with a cached token | `head` | No |
| 4 | `GET /v2/<repo>/manifests/<digest>` | `resolve` | Always first in `resolve` | `get`, `manifest_get` | Yes (with #5, one pull) |
| 5 | `GET /v2/<repo>/manifests/<platform digest>` | `resolve` | #4 returned an index or manifest list | `get`, `manifest_get` | Counted together with #4 |
| 6 | `GET /v2/<repo>/blobs/<config digest>` | `resolve` | After #4 or #5 | `get` | No |

`resolve` also starts anonymous and takes a token from #2 when challenged, exactly as above.
`head_digest` never makes #4, #5 or #6.

So, for Keel's check:

- **Unchanged tag, or a digest Keel already resolved**: 1–3 requests, `manifest_get = 0`, 0 pulls.
- **Newly published image**: 4–6 requests in total, `manifest_get` 1 or 2, 1 pull, once per
  process per digest (Keel's cache).

## Rules

1. **HEAD only for digests**: `head_digest` reads `Docker-Content-Digest`. If it is missing or not a
   valid digest, the result is `NoDigestHeader`. There is no `ETag` fallback and no GET fallback.
2. **Token flow**:
   - Start without credentials. On a 401, parse
     `WWW-Authenticate: Bearer realm="…",service="…",scope="…"`.
   - With `Credentials::Basic`, send them to the realm only, and only to an `https` realm (any realm
     under `test-util`'s base). With `Anonymous`, request an anonymous token.
   - The realm host may differ from the registry host (Docker Hub uses `auth.docker.io`).
   - Cache the token in memory per (realm, service, scope, username) until `expires_in` (default
     60 s) runs out.
   - A 401 or 403 after a token is `AuthFailed`. A 401 without a `Bearer` challenge is
     `AuthFailed`.
3. **Redirects**:
   - Blob GETs may redirect (Docker Hub sends blobs to a CDN). At most 3 redirects are followed,
     `https` only, and the `Authorization` header is dropped on a cross-host redirect.
   - Manifest requests and token requests do not follow redirects.
4. **Body limits**: manifests and indexes up to 4 MiB, config blobs up to 4 MiB, token responses
   up to 64 KiB. A larger body is `BadResponse`.
5. **Digest checks**: the bodies of #4, #5 and #6 are hashed with SHA-256. A mismatch with the
   requested digest is `BadResponse`.
6. **Platform selection** in `resolve`:
   - **Index or manifest list**: choose the entry with `platform.os` and `platform.architecture`
     equal to the requested platform, plus a matching `variant` when the entry has one. Skip
     entries whose platform is `unknown/unknown`, or whose annotations include
     `vnd.docker.reference.type`. No match is `NoImageForPlatform`.
   - **Single manifest**: read its config, and require the config's `os` and `architecture` to
     equal the requested platform. Otherwise `NoImageForPlatform`. `index_digest` is `None`, and
     `manifest_digest` is the requested digest.
7. **Timeouts**: 10 s per request (`Timeout`). A bound on a whole check is the caller's (Keel uses
   30 s).
8. **Error mapping**:

   | Error | Meaning |
   |---|---|
   | `AuthFailed` | 401 or 403 after a token, or a 401 with no Bearer challenge |
   | `NotFound` | 404 on a manifest |
   | `RateLimited { retry_after }` | 429, or Docker Hub's `TOOMANYREQUESTS` error code; `retry_after` from `Retry-After` |
   | `Timeout` | The per-request timeout was reached |
   | `Network` | Connection or TLS failure |
   | `BadResponse` | Anything malformed, oversized, mismatched, or an unexpected status |
   | `NoDigestHeader` | A HEAD response without a valid `Docker-Content-Digest` |
   | `NoImageForPlatform` | Rule 6 found no matching image |

9. **Logging**: the method, host, path and status of each request at `debug`. Never headers,
   never the token, never credentials.
10. **Counts**: every request made, including failed ones, is counted in the returned
    `RequestCounts`, so a 429 still shows its HEAD.

## Fake registry (`oci_registry::testing`, feature `test-util`)

`FakeRegistry` is an axum router served on `127.0.0.1:0`, with:

- `/token`, which issues a token, and requires `Basic` when the fixture is configured with
  credentials;
- `/v2/<repo>/manifests/<ref>` (HEAD and GET) and `/v2/<repo>/blobs/<digest>` from in-memory
  fixtures, with digests computed from the fixture bodies;
- a switch to omit `Docker-Content-Digest` on HEAD;
- a programmable status per path (for 401, 404, 429);
- a request log of `(method, path, had_authorization)`, which tests assert against.

Fixture builders cover:

- a single-manifest image, with labels;
- an index with amd64, arm64 and an attestation entry;
- an index without the requested platform;
- a single manifest whose config is for another architecture;
- a config without labels;
- a 429, with or without a `Retry-After` header;
- a HEAD without `Docker-Content-Digest`;
- a body whose digest doesn't match.

A test builds the client with `Client::with_base_url(ua, fake.url())`. The crate's own tests live in
`crates/oci-registry/tests/client.rs`. Keel's check tests use the same fake through a
dev-dependency on `oci-registry` with `features = ["test-util"]`. Nothing in any test reaches the
network.
