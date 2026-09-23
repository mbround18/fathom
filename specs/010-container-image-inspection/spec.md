# Feature Specification: Container Image Inspection

**Feature Branch**: `010-container-image-inspection`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Container Image Inspection feature described in docs/MVP.md §9a: a 'download and inspect layers' affordance on a container's image reference, without pulling the whole image — fetch the OCI manifest + config from the image's registry (using the same imagePullSecrets the cluster already has, when the registry is private) and show layer sizes, the image's build history (Cmd/commands per layer, from the OCI config's `history` field), total size, and base-image digest — the kind of thing `docker history`/`dive` give you today, surfaced in-app instead of requiring a shell. Lives off a Pod/Deployment's container spec as a new 'Image' tab/section per container in the existing Pod inspector, not its own nav section. Non-goals: CVE/vulnerability scanning, pulling/caching full image content, interactive/browser-based OIDC registry auth (service-account/secret-based auth only)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Inspect a running container's image without a shell (Priority: P1)

An operator looking at a Pod in Fathom wants to know what's actually inside one of its
containers' images — total size, how many layers, what command built each layer, and the base
image — without dropping to a terminal and running `docker history` or `dive` against a pulled
copy of the image.

**Why this priority**: This is the entire value proposition of the feature. Every other
capability (auth fallback, base-image detection, per-layer history) exists in service of this one
moment: an operator looking at a container and getting image internals in-app.

**Independent Test**: Open a running Pod's inspector for a container using a publicly reachable
image (e.g. an image from a public registry), select its "Image" tab, and confirm the view shows
the registry, repository, tag/digest, total image size, and a per-layer list with each layer's
size and build-history command, populated without the operator leaving Fathom.

**Acceptance Scenarios**:

1. **Given** a Pod with a container running a publicly reachable image, **When** the operator
   opens that container's "Image" tab in the Pod inspector, **Then** the tab shows the image's
   registry, repository, tag/digest, total size, and an ordered list of layers each with its size
   and the build-history command that produced it.
2. **Given** a Pod with multiple containers, **When** the operator switches between containers in
   the inspector, **Then** each container's "Image" tab shows that container's own image data, not
   a previously viewed container's data.
3. **Given** the operator has not yet opened a container's "Image" tab, **When** the Pod inspector
   is first opened, **Then** no registry fetch for that container's image has occurred (fetches are
   on-demand per tab, not eager for every visible pod/container).
4. **Given** the operator re-opens an "Image" tab for a container whose image digest was already
   fetched earlier in the session, **When** the tab loads, **Then** the data appears without a
   redundant registry round-trip for the same digest (data is remembered for the session).

---

### User Story 2 - Inspect an image in a private registry using the cluster's own pull secret (Priority: P1)

An operator's container runs an image from a private registry that requires authentication. The
Pod already successfully pulled it (via its `imagePullSecrets`), so the operator expects Fathom
to reuse that same credential to fetch manifest/config metadata, rather than being told the image
is inaccessible or being asked to enter new credentials.

**Why this priority**: A large share of production workloads use private registries. Without this,
the feature only works for public images and misses most real-world use, making it a P1 alongside
Story 1.

**Independent Test**: Deploy a Pod whose container image is hosted in a private registry and whose
`spec.imagePullSecrets` references a valid `kubernetes.io/dockerconfigjson` Secret for that
registry. Open the container's "Image" tab and confirm the manifest/config data loads
successfully using that Secret, with no new credential entry required from the operator.

**Acceptance Scenarios**:

1. **Given** a container image in a registry that rejects anonymous requests, **When** the
   operator opens that container's "Image" tab, **Then** the backend first attempts an anonymous
   fetch, receives an auth challenge, and retries using credentials resolved from the Pod's own
   `imagePullSecrets`, succeeding without any new credential input from the operator.
2. **Given** a Pod's `imagePullSecrets` reference a Secret that does not exist or does not grant
   access to the image's registry, **When** the operator opens the "Image" tab, **Then** the tab
   shows an explicit authentication-failure state naming the problem (e.g. "authentication failed
   for this registry"), not a blank panel or a generic crash.
3. **Given** a registry requires interactive/browser-based (device-code/OIDC) authentication that
   cannot be satisfied by a stored Secret, **When** the operator opens the "Image" tab, **Then** the
   tab shows an explicit "this registry's authentication isn't supported" state rather than
   attempting to open a browser flow or hanging indefinitely.

---

### User Story 3 - See a best-effort base image identification (Priority: P3)

An operator wants a quick, best-effort signal of what base image a container was built from (e.g.
`alpine`, `debian`, `distroless`), to help triage or compare containers, understanding this is a
heuristic and not a guarantee.

**Why this priority**: Useful for triage but not essential — the manifest/layer/history data from
Story 1 already delivers most of the feature's value on its own; base-image identification is a
nice-to-have layered on top.

**Independent Test**: Open the "Image" tab for a container built from a well-known public base
image (e.g. `alpine:3.19`) and confirm a best-effort base-image label appears, clearly marked as
best-effort/not guaranteed; open the tab for a container whose base image cannot be determined and
confirm the section shows an explicit "base image not determined" state rather than a wrong guess
presented as fact.

**Acceptance Scenarios**:

1. **Given** a container image whose layer digests share a prefix with a well-known public base
   image, **When** the operator opens the "Image" tab, **Then** a base-image identification is
   shown, visibly labeled as best-effort.
2. **Given** a container image whose layers don't match any recognized base image, **When** the
   operator opens the "Image" tab, **Then** the base-image section explicitly states it could not
   be determined, rather than omitting the section silently or showing a misleading guess.

---

### Edge Cases

- The image reference uses a mutable tag (e.g. `:latest`) rather than a digest: the tab resolves
  and displays the specific digest the manifest fetch actually returned, so the operator knows
  exactly which content was inspected, not just the tag.
- The image manifest is a multi-architecture manifest list: the backend resolves the entry
  matching the Pod's/node's platform (OS/architecture) before fetching config, and the tab shows
  data for that resolved platform-specific image, not the manifest list itself.
- The registry is unreachable (network error, timeout, DNS failure): the tab shows an explicit
  "registry unreachable" error state, not a blank or indefinitely-loading panel.
- The image no longer exists at the registry (deleted/retagged since the Pod started): the tab
  shows an explicit "image not found in registry" state.
- The container's image reference has no explicit registry host (e.g. bare `myimage:tag`,
  implying a default registry): the backend resolves it the same way the container runtime would
  (default registry convention), and the tab displays the resolved registry.
- A container is in `ImagePullBackOff`/`ErrImagePull` (the image was never successfully pulled by
  the node): the "Image" tab still attempts its own independent manifest fetch (using the same
  anonymous-then-secret auth flow) and shows whatever result that fetch produces — success, or an
  explicit error state — since the tab's fetch does not depend on the node having already pulled
  the image.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The Pod inspector MUST show a per-container "Image" tab/section for every container
  listed in the Pod, initContainers included.
- **FR-002**: Selecting a container's "Image" tab MUST fetch, on demand, only the image's OCI
  manifest and the config blob it references — the system MUST NOT fetch or download any layer
  content (blob bodies).
- **FR-003**: The system MUST NOT fetch manifest/config data for a container's image before the
  operator opens that container's "Image" tab (no eager/background prefetching for every visible
  Pod or container).
- **FR-004**: The "Image" tab MUST display: registry host, repository/image name, tag and/or
  digest as resolved from the manifest fetch, total image size, and an ordered list of layers each
  showing its size and its build-history command (from the image config's `history` field).
- **FR-005**: The system MUST attempt an anonymous registry request first; if the registry
  requires authentication, the system MUST retry using credentials resolved from the Pod's own
  `spec.imagePullSecrets`, without prompting the operator for new credentials or storing any new
  credential material.
- **FR-006**: The system MUST NOT provide any UI for entering, storing, or managing registry
  credentials — all credential resolution reuses Secrets already referenced by the Pod spec.
- **FR-007**: When the resolved manifest is a multi-architecture manifest list, the system MUST
  resolve and fetch the entry matching the Pod's/node's platform before presenting config/layer
  data.
- **FR-008**: When manifest/config data for a given image digest has already been fetched earlier
  in the session, re-opening that container's (or another container's, if it shares the same
  digest) "Image" tab MUST NOT re-fetch it, and MUST present the previously retrieved data.
- **FR-009**: The "Image" tab MUST show an explicit, distinguishable error state — not a blank or
  indefinitely-loading panel — for each of: registry unreachable, image not found, authentication
  failed, and registry authentication method unsupported (e.g. interactive/OIDC-only registries).
- **FR-010**: The "Image" tab MAY show a best-effort base-image identification derived from shared
  layer digests against a set of well-known public base images; this identification MUST be
  visibly labeled as best-effort, and its absence (when no match is found) MUST be shown as an
  explicit "not determined" state, never a fabricated guess presented as fact.
- **FR-011**: All registry network calls (manifest fetch, config blob fetch, credential
  resolution) MUST be performed by the backend; the UI MUST NOT make direct registry HTTP calls,
  consistent with the rest of Fathom's backend-owns-the-cluster architecture.
- **FR-012**: This feature MUST NOT scan or report on vulnerabilities/CVEs for the inspected
  image.
- **FR-013**: This feature MUST NOT pull, download, or locally cache any image layer content
  (blob bodies) — only manifest and config metadata are ever fetched.
- **FR-014**: This feature MUST NOT implement or attempt interactive/browser-based (device-code or
  OIDC) registry authentication flows; registries requiring such flows MUST fall into the
  "authentication method unsupported" error state (FR-009) rather than being partially supported.
- **FR-015**: This feature is read-only: it MUST NOT alter the Pod, its containers, or the
  registry/image in any way.

### Key Entities

- **Image Reference**: The registry host, repository name, and tag and/or digest identifying a
  container's image, as resolved from a container's `image` field.
- **Image Manifest**: The OCI manifest for a resolved image reference (or the platform-resolved
  entry of a manifest list) — references the image config blob and an ordered list of layer
  descriptors (digest, size, media type).
- **Image Config**: The OCI image config blob referenced by the manifest — carries creation date,
  labels, environment variables, entrypoint/cmd, and the per-layer `history` entries (build
  command per layer).
- **Layer**: One entry in an image's layer list — has a digest, a size, and (via the config's
  `history`) the build command that produced it.
- **Registry Credential**: A `kubernetes.io/dockerconfigjson`-type Secret already referenced by a
  Pod's `spec.imagePullSecrets`, reused (never newly created or stored) to authenticate a
  manifest/config fetch when anonymous access is rejected.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can view a running container's image size, layer count, and per-layer
  build history within 5 seconds of opening its "Image" tab (network conditions permitting),
  without leaving Fathom or opening a shell.
- **SC-002**: For containers backed by private-registry images whose Pod already has a working
  `imagePullSecrets` reference, image inspection succeeds without any additional operator input in
  100% of cases where the underlying Secret still grants registry access.
- **SC-003**: Every failure condition (unreachable registry, missing image, auth failure,
  unsupported auth method) results in a specific, readable explanation shown in the "Image" tab —
  never a blank panel, an indefinite spinner, or an uncaught crash.
- **SC-004**: Re-opening a previously viewed container's "Image" tab within the same session shows
  its data without a repeated network fetch for the same image digest.
- **SC-005**: No image layer content (blob bodies) is ever downloaded by this feature, regardless
  of image size — verified by the total data transferred for an "Image" tab load being bounded by
  manifest + config size only, not the sum of layer sizes.

## Assumptions

- The container runtime/registry ecosystem in scope follows the OCI Distribution and OCI Image
  Format specifications (or the compatible Docker v2 manifest/config formats most registries still
  serve); registries that only implement older or nonstandard protocols are out of scope.
- Credential resolution reuses exactly the Secrets already referenced by the Pod's own
  `spec.imagePullSecrets`; this feature does not resolve registry credentials from any other
  source (e.g. node-level Kubelet credential providers, ambient cloud-provider credentials), since
  those are not already visible to Fathom's existing Pod-spec reads.
- Base-image identification (User Story 3) is best-effort, sourced from a small built-in list of
  well-known public base images matched by shared layer digest — it is not a comprehensive or
  guaranteed-accurate image-provenance system.
- Manifest/config data fetched during a session is cached in memory only, keyed by image digest,
  for the lifetime of the session; it is not persisted to disk and is not shared across separate
  Fathom sessions or cluster connections.
- "Total image size" means the sum of all layer sizes as reported by the manifest, plus the config
  blob size — the same figure registries and tools like `docker images`/`dive` typically report —
  not on-disk size after decompression.
- A registry-client crate that already implements the OCI Distribution spec (e.g. `oci-client`) is
  added to the backend's dependencies for this feature, since no such crate exists in the project
  today; this is a new, minimal dependency addition consistent with the project's existing
  practice of using public spec-compliant crates (`kube`, `tonic`) rather than hand-rolling
  protocol clients.
