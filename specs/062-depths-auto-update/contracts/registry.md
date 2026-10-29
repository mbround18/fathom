# Contract: registry client, port and requests (062)

Three layers:

1. `crates/oci-registry`: the shared client, used by Keel (061) and Depths (062). No app types.
2. The `Registry` port in `depths-core`: what the pure check logic sees.
3. depths-server's `updates/registry.rs` and `updates/creds.rs`: the port over the crate, and
   Depths' own reading of `config.json`.

## 1. Shared crate: `crates/oci-registry`

The crate's contract is
[specs/061-keel-update-notice/contracts/registry.md](../../061-keel-update-notice/contracts/registry.md):
its `Cargo.toml`, API, reference table, request list, rules and fake registry. That file is the one
source; this spec does not restate or extend it. Whichever of 061 and 062 is implemented first
builds the crate to it; the other uses it as it is.

What Depths relies on, by name:

- `ImageRef::parse`, `registry()`, `repository()`, `reference()` (`Reference::Tag | Digest`),
  `with_digest(Digest)`; `Digest::parse`, `as_str()`. `registry()` is the API host, so Docker Hub
  is `registry-1.docker.io`.
- `Credentials::{Anonymous, Basic { username, password: SecretString }}`, with a redacting `Debug`.
- `Platform { os, architecture, variant }`. Depths builds it from the daemon (section 3), not
  `Platform::current()`, since the daemon's platform is what runs.
- `Client::new(user_agent)` (`https` only); `head_digest(&ImageRef, &Credentials) ->
  Counted<Digest>` (HEAD only, never a GET fallback); `resolve(&ImageRef by digest, &Platform,
  &Credentials) -> Counted<Resolved>` with `Resolved { index_digest: Option<Digest>,
  manifest_digest, config_digest, labels, user, created }`.
- `Counted<T> { result, requests: RequestCounts { head, get, manifest_get } }`: every call reports
  the requests it made, failures included. Depths feeds its metrics from it (section 3).
- `RegistryError::{InvalidReference, NotByDigest, AuthFailed, NotFound, RateLimited, Timeout,
  Network, BadResponse, NoDigestHeader, NoImageForPlatform}` with `code()`.
- Feature `test-util`: `oci_registry::testing::FakeRegistry` (axum on `127.0.0.1:0`, request log
  of `(method, path, had_authorization)`) and `Client::with_base_url(user_agent, url)`.
  depths-server enables it in `[dev-dependencies]` only.

The crate pins its own dependency versions, as `web-auth` does: the workspace has no
`[workspace.dependencies]` table. It adds no new package to the lockfile.

## 2. Port (`depths-core`, agnostic, no I/O)

The port returns boxed futures, so it can be held as `Arc<dyn Registry>` without adding
`async-trait` to normal dependencies. It takes no credentials: the server's implementation looks
them up per registry from `config.json` (section 3).

```rust
pub type RegistryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RegistryError>> + Send + 'a>>;

pub trait Registry: Send + Sync {
    /// HEAD the tag. Never GETs.
    fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String>;
    /// Resolve a digest of the reference's repository for a platform (counted as one pull by Docker Hub).
    fn resolve<'a>(&'a self, r: &'a ImageRef, digest: &'a str, p: &'a Platform) -> RegistryFuture<'a, Resolution>;
}
pub struct Platform { pub os: String, pub architecture: String, pub variant: Option<String> }
pub struct Resolution {
    pub index_digest: Option<String>, pub manifest_digest: String, pub config_digest: String,
    pub labels: BTreeMap<String, String>, pub user: Option<String>, pub created: Option<String>,
}
pub enum RegistryError {
    AuthFailed,
    NotFound,
    RateLimited,
    Unreachable(String),            // timeout, network
    BadResponse(String),
    NoDigestHeader,                 // "can't check": never pulled to find out
    NoImageForPlatform,
    CredentialsUnsupported(String), // "can't check": helper or identity token in config.json
}
```

`ImageRef` here is depths-core's own (data-model.md), parsed purely for the exclusion rules. The
resolution and comparison logic (research R1) is a pure function over this port and lives in
`depths-core::updates::check`. The fake registry in core tests implements the port and counts
`head_digest` and `resolve` calls.

Every `RegistryError` becomes `check_failed` with a reason, and the service keeps its last known
state (FR-005). `NoDigestHeader` and `CredentialsUnsupported` read "can't check: …"; only
`RateLimited` and `Unreachable` start the per-registry backoff (research R5).

## 3. Depths' implementation (`apps/depths/server/src/updates/`)

- `registry.rs`: `OciRegistry` implements the port over one `oci_registry::Client`, built once
  with `Client::new("depths/<version> (update-check)")`. Each call adds the `RequestCounts` of its
  `Counted` result to `depths.updates.registry_requests` (research R9). For each check pass it is
  given a credentials snapshot from `creds.rs`.
  - Reference: `oci_registry::ImageRef::parse(&core_ref.to_string())`. A unit test runs the
    data-model's reference table through both parsers and requires the same repository and
    reference, and the same registry once `docker.io` is read as `registry-1.docker.io`.
  - Error mapping: `AuthFailed`, `NotFound`, `RateLimited`, `NoDigestHeader`,
    `NoImageForPlatform` one to one; `Timeout` and `Network` → `Unreachable`; `BadResponse`,
    `InvalidReference` and `NotByDigest` → `BadResponse` (the last two are Depths bugs, logged).
  - The crate does not report `Retry-After`; the backoff is Depths' own (research R5).
- Platform: the host's `os/architecture/variant` from `docker info` (`OSType`, `Architecture`
  normalised: `x86_64`→`amd64`, `aarch64`→`arm64` + variant `v8`), or the service's `platform:`.
- Registries Docker reaches over plain `http` (loopback or `insecure-registries`) show "can't
  check"; the client only speaks `https` outside tests.

## Credentials file (`$DOCKER_CONFIG/config.json`, `creds.rs`)

Read per check pass, never cached across passes, never written.

| Field | Used |
|---|---|
| `auths.<key>.auth` | base64 `user:password` → `Credentials::Basic` |
| `auths.<key>.username` + `password` | `Credentials::Basic` |
| `auths.<key>.identitytoken` | Not supported: `CredentialsUnsupported` "identity tokens are not supported; log in with a username and password". |
| `credsStore`, `credHelpers.<host>` | Not supported: `CredentialsUnsupported` "credential helper `<name>` is not supported in the Depths container". |
| anything else | ignored |

Key matching: exact host, `https://host`, `https://host/v1/`, `http://host`. Docker Hub
(`docker.io`) matches `https://index.docker.io/v1/`, `index.docker.io`, `docker.io`,
`registry-1.docker.io`. A host with an entry uses its credentials; a host without one is
`Anonymous`. A `credHelpers` entry for the host, or a `credsStore` with no `auths` entry for it,
is `CredentialsUnsupported`.

A missing file means anonymous. An unreadable file: checks run anonymously, the registries panel
on Updates shows "docker config unreadable: <path>", and private images fail with `AuthFailed`. The file's
contents never appear in logs, spans, audit entries, the protocol or Depths' state.

**For users:** private registries need a plain `config.json` mounted into the Depths container
and `DOCKER_CONFIG` set to its directory (contracts/config.md). Spec 050's pulls of private images
need the same mount.

## Request budget (tested, SC-007)

With the fake port and a fake clock, 30 stacks × one service each, 6 h interval, 7 days. The core
suite counts port calls; the crate's request table (061 contract, "Requests") pins how many HTTP
requests one call makes (`head_digest`: one or two HEADs and a token GET when no token is cached;
`resolve`: up to two manifest GETs, which Docker Hub counts as one pull, and one blob GET).

| Scenario | `head_digest` (HEAD) | `resolve` (manifest GETs) | blob GET |
|---|---|---|---|
| No tag moves | 30 × 28 = 840 | 0 | 0 |
| One tag moves once, multi-arch | 840 | 1 (2 GETs, one counted pull), once | 1 |
| Same tag moves, 10 stacks share the image | 840 | 1 (2 GETs), once | 1 |
| Running side under containerd store needs its own platform manifest | 840 | +1 per old index, once | +1 |
| Registry 429 for a day | fewer (backoff) | 0 | 0 |
| Registry sends no digest on HEAD | 840 | 0 (`NoDigestHeader`, "can't check") | 0 |

The integration test (`DEPTHS_IT_DOCKER=1`) repeats the first two rows against a local
`distribution` registry, reached through `Client::with_base_url` (depths-server's dev-dependency
on `oci-registry` with `test-util`), and reads its access log and the `RequestCounts`.
