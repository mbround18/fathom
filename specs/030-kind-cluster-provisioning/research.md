# Research: In-App Local Kind Cluster Provisioning

## R1: Should Fathom vendor/bundle the `kind` binary?

**Decision**: No — for v1, detect an existing `kind` install on `PATH` and, if missing, show
install guidance (link to kind's install docs) rather than bundling the binary.

**Rationale**: kind's release binaries are per-OS/arch and are updated independently of
Fathom's release cadence; bundling would mean Fathom either (a) ships a pinned kind version
that drifts from what `kind` itself recommends pairing with a given Kubernetes/Docker version,
or (b) adds a self-update mechanism for a vendored binary, which is meaningfully more
implementation surface than this feature's scope justifies. The constraint in the task
description explicitly frames vendoring as a decision to evaluate, not a default — and the
detect-and-guide pattern is also exactly what's already required for the container runtime
(Docker/Podman), which is far too large to vendor. Keeping both prerequisites on the same
"detect, don't install" footing keeps the UX consistent and the backend simpler.

**Alternatives considered**:
- *Vendor the kind binary in the app bundle*: rejected for v1 per above; revisit only if user
  feedback shows the manual-install step is a major adoption blocker.
- *Auto-download kind on first use (not bundled, but fetched at runtime)*: rejected — this is a
  silent-install behavior close to what the task explicitly rules out ("do NOT attempt to
  silently install kind/Docker/Podman"), and introduces network-dependency and checksum-
  verification concerns out of scope here.

## R2: Container runtime detection (Docker vs. Podman)

**Decision**: Detect Docker first (`docker version` / `docker info` style ping via
`Command::new("docker").arg("info")`), then Podman if Docker is absent
(`Command::new("podman").arg("info")`). Report a single unified "container runtime" status
(installed & running / installed & not running / not installed) rather than exposing Docker and
Podman as two separate rows, since `kind` itself only needs *some* working runtime.

**Rationale**: `kind` supports both Docker and Podman as its provider; the spec's user-facing
requirement (FR-002) only needs to know "do I have a runtime that works," not a full breakdown.
A unified status is simpler for both the panel and for the pre-creation gate. `kind` itself
already understands the `KIND_EXPERIMENTAL_PROVIDER` environment variable to pick Podman when
present; Fathom will set that when spawning `kind` if Docker was not detected but Podman was, so
the tool's own provider selection logic is reused rather than reimplemented.

**Alternatives considered**: Probing via a Docker/Podman SDK crate — rejected as unnecessary
weight; a subprocess call mirrors the existing `helm_binary_available()` pattern
(`src/backend/helm.rs:570-575`) and needs no new dependency.

## R3: Long-running progress streaming pattern

**Decision**: Reuse the `DrainProgress`/`NodeActionFinished` request-id-keyed streaming pattern
(`src/protocol/v1/event.rs:467-478`, backend emitter `src/backend/nodes.rs:536-554`, frontend
reducer `ui/src/store/handlers/stream.ts:108`, progress view `NodeActionsPanel.tsx:414`) rather
than inventing a new progress mechanism. New events: `StreamEvent::KindProvisionProgress
{ request_id, stage: String, message: String }` emitted as `kind create cluster --config -`
writes lines to stderr (kind logs its own stage progress there — e.g. "Ensuring node image...",
"Preparing nodes...", "Writing configuration...", "Starting control-plane...", "Installing CNI...",
"Installing StorageClass...", "Waiting ≤ 6m0s for control-plane..."), and a terminal
`StreamEvent::KindClusterActionFinished { request_id, ok, error }` mirroring
`NodeActionFinished`. A `BackendCmdV1::CancelCreateKindCluster { request_id }` command mirrors
`CancelDrainNode`, aborting the spawned child process via the same kind of action-registry
pattern used for node actions (`state.node_actions`), generalized to a `state.kind_actions`
(or reuse of a shared long-running-action registry if one exists by implementation time).

**Rationale**: No Helm-install-progress or AI-agent-streaming precedent exists in this codebase
(confirmed by code search); `DrainProgress` is the only true multi-step, request-id-keyed
progress-streaming precedent, and it already satisfies SC-004 ("progress update at least every
few seconds") because kind itself logs a line roughly every few seconds during creation.

**Alternatives considered**: Polling `kind get clusters` on an interval from the frontend during
creation — rejected because it violates Principle II (frontend must not infer state by polling a
proxy for the real operation) and doesn't give fine-grained "still working" feedback the way
consuming kind's own progress lines does.

## R4: Registering the new cluster into spec 012's context federation

**Decision**: On successful `kind create cluster`, `kind` itself already merges a new context
into the user's default kubeconfig (`~/.kube/config`, or `$KUBECONFIG` if set) unless
`--kubeconfig` is passed to redirect it elsewhere. Fathom's create-cluster backend will let
`kind` write to the same kubeconfig path Fathom already reads via `load_kubeconfig()` /
`load_kubeconfig_from_env()` / `load_kubeconfig_from_home()` (`src/backend/kube.rs:19-77`), and
will then call the existing `load_and_send_contexts(...)` routine — the same one
`BackendCmdV1::RefreshContexts` already triggers — to re-read the kubeconfig and emit the
updated context list. No new context-registration command or Fathom-owned registry is created;
this directly satisfies the task's instruction not to invent a parallel mechanism, since spec
012's `RefreshContexts` / `FederationContextSwitcher` already becomes aware of the new context
purely because `kind` wrote it into the same file Fathom already watches.

**Rationale**: Confirmed via code search (`src/backend/cmd/cluster.rs:24-33`,
`src/backend/kube.rs`) that there is no existing "register a brand-new context" command —
contexts are always sourced by re-reading the kubeconfig file from disk. The simplest, most
consistent design is therefore: let `kind` (which already knows how to correctly merge a
context/cluster/user triple into a kubeconfig) do the merge, then trigger the same refresh path
already used elsewhere. Deletion is symmetric: `kind delete cluster` already removes its
context/cluster/user entries from the kubeconfig, so a follow-up `RefreshContexts`-equivalent
reflects the removal without extra bookkeeping.

**Alternatives considered**: Have Fathom parse `kind`'s generated kubeconfig separately and
merge it into the main kubeconfig itself via `kube::config::Kubeconfig` merge APIs — rejected
as unnecessary duplication of what `kind` already does correctly and safely; only fall back to
this if a future need arises to keep the kind cluster's kubeconfig fully separate (e.g.
`--kubeconfig <isolated-path>`), which is not required by this spec.

**Constitution VI note**: Because `kind` writes directly to the shared kubeconfig file, Fathom
must NOT have any other in-flight kubeconfig write during that window; the create/delete
kind-cluster mutation is serialized against any other Fathom-initiated kubeconfig write (there
are none today, but this is documented so a future feature doesn't introduce an unguarded
concurrent writer).

## R5: Cluster name validation

**Decision**: Validate the user-provided cluster name against the same rules kind itself
enforces for cluster names, which are RFC 1123 DNS label-compatible: 1-63 characters, lowercase
alphanumeric or `-`, must start and end with an alphanumeric character. Reject empty strings and
names that collide with a cluster already returned by `kind get clusters`. Validation happens
in Rust (source of truth, since the backend is what constructs the subprocess argument) and is
mirrored in the frontend for inline UX feedback (FR-006), but the backend re-validates
regardless of what the frontend sent, per Principle II (frontend cannot be trusted as the
enforcement boundary).

**Rationale**: This matches the task's explicit security requirement (never build a raw shell
string from user input; validate before passing as a CLI argument) and reuses an already
well-understood, boring validation rule (DNS-1123 label) rather than inventing new naming rules.

**Alternatives considered**: Allowing arbitrary names and relying solely on `kind`'s own
rejection — rejected because it pushes invalid input further downstream than necessary and
delays feedback past the point FR-006 requires (rejection before confirmation is reachable).

## R6: Cluster profiles for v1

**Decision**: Ship exactly one built-in profile beyond "bare": `bare` (default, single
control-plane node, no extras) is the only profile in v1. The "profile" concept in the data
model and protocol is designed to be extensible (a `KindClusterProfile` enum/string with a
description), but only `bare` ships now. A `custom-yaml` pseudo-profile is added as the v1
raw-config affordance (see R7) rather than a curated "batteries-included operators" profile.

**Rationale**: The task explicitly says the exact operator bundle for a "with common operators"
profile is a decision that could be shared with the separate `scripts/dev-cluster` dev/CI
harness effort, which this worktree cannot see the in-progress design of. Shipping a
placeholder curated profile now risks diverging from whatever that harness settles on. Flagging
this as a **possible shared asset**: if/when `scripts/dev-cluster` lands a "profile" concept
(e.g. bare vs. cert-manager+Gatekeeper), a future feature should evaluate whether its profile
definitions (a list of Helm charts/manifests to apply post-create) can be reused as the source
of Fathom's non-bare in-app profiles, rather than Fathom defining its own separate list. This is
a follow-up integration opportunity, not something this feature blocks on.

**Alternatives considered**: Shipping a curated "cert-manager + Gatekeeper" profile now —
deferred to avoid duplicating/diverging from the other in-flight harness's asset.

## R7: Raw kind-config YAML paste-in affordance

**Decision**: Include a minimal v1 affordance: an optional "Advanced: paste kind-config YAML"
toggle on the create form that, when enabled, replaces the name+profile inputs' generated
config with a raw YAML textarea (still requiring the same validated `name` field separately,
since `kind create cluster --name` is passed explicitly rather than only relying on
`metadata.name` inside the YAML — kind's own config schema does not carry a top-level cluster
name field, so Fathom keeps name entry outside the pasted YAML to keep validation centralized).
This reuses the plain textarea + basic YAML-parse-validate helper pattern already present in
`ui/src/resources/yaml.ts`, but as a lean new small component rather than reusing
`YamlEditTab.tsx` wholesale, since that component is tightly coupled to diffing an existing
live cluster object (`ObjectRef`/`raw` props) which doesn't apply to a not-yet-existing kind
cluster config.

**Rationale**: The task frames this as "a reasonable v1 affordance if it fits cleanly... reusing
the existing raw-YAML editor component... otherwise defer it." Direct reuse of `YamlEditTab`
does not fit cleanly (it's built around editing/diffing a live resource), but the smaller
YAML-parsing utility it depends on (`ui/src/resources/yaml.ts`) does fit cleanly, so a thin new
textarea component built on that utility is the right-sized affordance rather than a full defer.

**Alternatives considered**: Fully deferring any raw-YAML path to a future release — rejected
because the marginal cost of a plain textarea + existing parse helper is small and directly
requested as a "reasonable v1 affordance," but scoped down from reusing the heavier diff-aware
component.

## R8: Backend process invocation safety

**Decision**: All `kind`/`docker`/`podman` invocations use `tokio::process::Command::new(bin)`
with `.arg(...)` calls for each argument (mirroring `src/backend/helm.rs:1288-1298`), never
`sh`/`cmd.exe` and never a formatted/interpolated string. The validated cluster name (R5) is the
only user-controlled value that reaches a `kind` argument; it passes through the same validator
regardless of whether it originated from the simple form or was extracted from a pasted YAML
config's `--name` companion field.

**Rationale**: Directly satisfies the task's explicit security requirement and Principle II.
Matches the codebase's one existing convention for shelling out (Helm invocation).
