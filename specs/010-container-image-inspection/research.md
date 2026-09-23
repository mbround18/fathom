# Research: Container Image Inspection

## R1: Registry-client crate

**Decision**: `oci-client` (crates.io), latest stable at time of writing 0.18.0, published
2026-09-18. This is the actively maintained successor to `oci-distribution`, developed under the
ORAS project (CNCF-adjacent), Apache-2.0 licensed, ~8M total downloads. Confirmed via
`grep -i "oci\|registry" Cargo.toml` that no OCI/registry crate exists in the workspace today.

**Rationale**: Implements the OCI Distribution spec (manifest/blob pull, auth challenge handling,
manifest-list/index resolution) so Fathom does not hand-roll registry HTTP/auth protocol code,
consistent with existing practice (`kube`, `tonic`). Exposes `Client::pull_manifest_and_config`
plus lower-level `pull_manifest`/`fetch_blob` calls, and an `Auth`/`RegistryAuth` enum that
supports anonymous and basic (username/password or token) credentials — a direct fit for the
anonymous-then-imagePullSecrets-fallback flow required by FR-005.

**Alternatives considered**:
- Hand-rolled reqwest client against the OCI Distribution spec — rejected: reinvents a
  spec-compliant client that already exists and is maintained, against user's standing
  crates/API-spec-first constraint.
- `dkregistry` — less actively maintained, smaller community, no manifest-list resolution helper.
- Shelling out to `docker`/`skopeo` — rejected: adds an external binary dependency, violates
  "backend owns the cluster" spirit of using typed Rust clients, and isn't guaranteed present on
  Tier 1 platforms.

## R2: Credential resolution (imagePullSecrets)

**Decision**: Reuse the existing `kube::Api<Secret>` read pattern already used in
`src/backend/helm.rs` (release-storage Secret reads). On a container's "Image" tab open:
1. Re-fetch the Pod (`Api<Pod>::get`) to read `spec.containers[i].image` and
   `spec.imagePullSecrets` (list of `LocalObjectReference { name }`).
2. Attempt an anonymous manifest pull via `oci-client`.
3. If the registry responds with a 401/auth-challenge, iterate `spec.imagePullSecrets`, read each
   named Secret (`Api<Secret>::namespaced(client, pod_namespace)`) of type
   `kubernetes.io/dockerconfigjson`, parse the `.dockerconfigjson` key's JSON
   (`{"auths": {"<registry>": {"username", "password", "auth"}}}`), find the entry matching the
   resolved registry host, and retry the pull with `RegistryAuth::Basic(username, password)`.
4. If no secret matches or all attempts fail with auth errors, surface `AuthenticationFailed`.
5. If any error surfaces that indicates a registry demanding OAuth2/device-code/browser flow
   (distinct WWW-Authenticate scheme we cannot satisfy with basic/token creds derived from a
   dockerconfigjson entry), surface `AuthMethodUnsupported`.

No new credential storage or UI is introduced (FR-006); credentials never leave the backend
process and are never logged (Constitution VI).

## R3: Multi-arch manifest lists

**Decision**: When `oci-client::Client::pull_manifest` returns an
`OciImageManifest`-vs-`OciImageIndex` (or `ImageIndex`/manifest list), select the entry whose
`platform.os`/`platform.architecture` matches the Pod's scheduled node
(`pod.status.host_ip`/`node_name` → the node's `status.nodeInfo.operatingSystem/architecture` via
`Api<Node>`), falling back to `linux/amd64` when the node's arch can't be resolved (e.g. Pod not
yet scheduled), per Edge Cases in spec.md.

## R4: Session-scoped caching

**Decision**: In-memory `HashMap<String /* digest */, CachedImageInspection>` on `BackendState`,
guarded by the same interior-mutability pattern already used for other per-session backend state
(e.g. `log_stream`). Never persisted to `~/.fathom/` (Constitution VI — this is ephemeral session
data, not user data). Keyed by resolved manifest digest so two containers sharing an image only
fetch once (FR-008).

## R5: Base-image best-effort identification (US3, P3)

**Decision**: A small built-in static table (`&[(&str, &[&str])]`) mapping well-known base images
(`alpine:3.19`, `alpine:3.20`, `debian:12-slim`, `distroless/static`, `ubuntu:24.04`, …) to their
publicly known base layer digest(s), refreshed manually as a static asset — no network fetch for
this table. Match by checking whether the inspected image's earliest N layer digests are a prefix
of (or contain) a table entry's digest set. Table entries are illustrative/best-effort per
FR-010/spec Assumptions, not a comprehensive provenance system; ships with a handful of common
entries and is clearly documented as extensible.

## R6: Test isolation / no-live-network convention

**Decision**: Repo has no prior "live registry" test convention (checked — no existing
`#[ignore]`-gated network tests found referencing a real registry). This feature establishes one:
unit tests for manifest/config JSON parsing and the auth-fallback state machine run entirely
against fixture bytes / a local `mockito`-style HTTP mock (no crate added solely for this if
`oci-client`'s own test doubles suffice; otherwise add `wiremock` or `httpmock` as a dev-dependency
only). Any test that would need a real registry round-trip is marked `#[ignore]` with a doc comment
explaining why, and is not part of the default `cargo test` run.
