# Feature Specification: Managed-Cloud Auth Plugins (EKS / GKE / AKS)

**Feature Branch**: `035-cloud-auth-plugins`
**Created**: 2026-09-24
**Status**: Draft
**Input**: docs/MVP.md §14 Tier D P3 — native credential/context plumbing for managed-cloud auth
plugins by invoking each provider's official exec plugin (`aws eks get-token` /
`aws-iam-authenticator`, `gke-gcloud-auth-plugin`, `kubelogin`), with detection, clear
missing-plugin errors, token-cost caching, and sandbox/security considerations.

## Background (informative)

`kube-rs` already honors a kubeconfig `users[].user.exec` entry (client.authentication.k8s.io)
by spawning the command and using the returned `ExecCredential`. What is missing is everything
around it: Fathom cannot tell an operator *why* a plugin-backed context failed (missing binary,
expired SSO, wrong `apiVersion`), and two call sites amplify the cost: the fleet poller
(`src/backend/fleet.rs::poll_inner`) builds a fresh `Client` every 30s per context, and the
context reachability probe (`context_status.rs`) builds one per probe. Each fresh client
re-runs the plugin (a process spawn, often a network round trip to STS/IAM/Entra).
Supersedes the P3 story left open in 012-incluster-daemon-federation (User Story 3).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Actionable errors for plugin-backed contexts (Priority: P1)

An operator selects (or the sidebar probes) a context whose kubeconfig uses an exec plugin. If
the plugin binary is missing, not executable, or exits non-zero, Fathom names the plugin, the
cloud, and the context, instead of a generic connection error.

**Why this priority**: The dominant failure today; needs no caching or new UI beyond error text.

**Independent Test**: Kubeconfig fixture with `command: gke-gcloud-auth-plugin` and an empty
`PATH`; assert the emitted status/error identifies plugin, cloud (GKE), and context.

**Acceptance Scenarios**:
1. **Given** a context whose exec `command` is not resolvable on `PATH`, **When** it is selected,
   probed, or polled by fleet, **Then** the error kind is `pluginMissing` and names the command,
   the inferred provider, the context, and the official install hint.
2. **Given** the plugin exits non-zero, **Then** the error is `pluginFailed` carrying the
   plugin's stderr, truncated and secret-redacted (Principle I: verbatim, minus credentials).
3. **Given** the plugin returns an expired `ExecCredential` or the API rejects it (401),
   **Then** the error is `credentialExpired` with a provider-specific re-login hint
   (`aws sso login`, `gcloud auth login`, `kubelogin`/`az login`), never auto-executed.
4. **Given** a static-token/cert context, **Then** behavior and messages are unchanged.

### User Story 2 - Detect and label cloud plugins per context (Priority: P2)

The operator can see, per context, that it uses a cloud exec plugin, which provider, and whether
the plugin is installed — before connecting.

**Independent Test**: Kubeconfig with EKS, GKE, AKS, and static contexts; assert detection
results (provider, command, installed) for each.

**Acceptance Scenarios**:
1. **Given** contexts using `aws`/`aws-iam-authenticator`, `gke-gcloud-auth-plugin`, `kubelogin`
   (matched by command basename, incl. `.exe` on Windows), **Then** each is classified
   `eks` / `gke` / `aks`; any other exec command is `genericExec`; none is `none`.
2. **Given** a classified context, **Then** the sidebar tooltip/inspector shows provider, command
   and installed/missing status; detection never spawns the plugin (only resolves the binary).
3. **Given** a missing plugin, **Then** the inspector shows the install hint and a copyable
   command; Fathom never installs anything.

### User Story 3 - Token-cost caching (Priority: P2)

Fathom does not re-run the plugin every 30s per context when the previous credential is still
valid.

**Independent Test**: Counting fake plugin (increments a file); run N fleet polls inside the
credential lifetime; assert one invocation. Advance past expiry; assert re-invocation.

**Acceptance Scenarios**:
1. **Given** a plugin returned `expirationTimestamp`, **When** subsequent polls occur before
   `expiry - skew` (default 60s), **Then** the cached credential is reused, plugin not spawned.
2. **Given** a response without expiry, **Then** it is cached for a bounded default (5 min) TTL.
3. **Given** the API server returns 401 with a cached credential, **Then** the entry is
   invalidated and the plugin re-invoked exactly once for that attempt.
4. **Given** concurrent polls/probes for one context, **Then** only one plugin invocation is in
   flight (single-flight); others await its result.
5. **Given** the app exits, **Then** nothing is persisted: cache is in-memory only.

### User Story 4 - Safe plugin execution (Priority: P3)

Plugin execution is bounded and cannot leak credentials or be abused via a hostile kubeconfig.

**Acceptance Scenarios**:
1. **Given** a plugin that hangs, **Then** it is killed at a timeout (default 20s) and reported
   as `pluginTimeout`.
2. **Given** a kubeconfig exec entry with a command not in the provider allowlist, **Then**
   Fathom follows the operator's `execPolicy` (default `allowKnownOnly`; `allowAll` opt-in per
   context) and otherwise reports `pluginBlocked` naming the command. Rationale: opening an
   untrusted shared kubeconfig must not run arbitrary binaries (relevant to 014 shared profiles).
3. **Given** any plugin output, **Then** tokens never appear in logs, protocol events, UI, MCP
   output, or exported diagnostics (Principle VI); only expiry and provider are surfaced.
4. **Given** interactive plugins (`interactiveMode: Always`), **Then** Fathom does not attach a
   TTY; it reports `pluginInteractiveUnsupported` with guidance to log in via the provider CLI.

### Edge Cases
- Plugin on `PATH` in the login shell but not in a GUI-launched app's `PATH` (macOS/Windows
  Finder/Start launch): resolve using an augmented search path (`/opt/homebrew/bin`,
  `/usr/local/bin`, `~/.local/bin`, `%ProgramFiles%\...`), and mention it in the error.
- `apiVersion` mismatch (`v1alpha1` vs `v1beta1` vs `v1`): pass through; surface plugin error.
- Same plugin/args/env across contexts: cache key includes command, args, env, cluster info,
  so distinct clusters never share a token.
- Context removed/changed on kubeconfig reload: its cache entry is dropped.
- `provideClusterInfo: true`: cluster info is passed as `KUBERNETES_EXEC_INFO` as kubectl does.

## Requirements *(mandatory)*

- **FR-001**: The backend MUST classify every context's user auth as `eks|gke|aks|genericExec|none`.
- **FR-002**: The backend MUST resolve the exec command binary without spawning it, and report installed/missing.
- **FR-003**: Connect, probe, and fleet poll MUST map plugin failures to typed errors (`pluginMissing`, `pluginFailed`, `pluginTimeout`, `credentialExpired`, `pluginBlocked`, `pluginInteractiveUnsupported`).
- **FR-004**: Errors MUST name plugin, provider, context, and hint; stderr MUST be truncated and redacted.
- **FR-005**: The backend MUST cache `ExecCredential` in memory keyed per plugin invocation identity, honoring expiry with skew, a default TTL, single-flight, and 401 invalidation.
- **FR-006**: Fleet and probe MUST obtain credentials through the shared cache so a fresh `kube::Client` per poll does not re-spawn the plugin.
- **FR-007**: Plugin execution MUST enforce a timeout and MUST NOT allocate a TTY.
- **FR-008**: Unknown exec commands MUST be gated by `execPolicy` (default known-only).
- **FR-009**: Tokens MUST NOT be logged, emitted in events, stored on disk, or surfaced to UI/MCP.
- **FR-010**: Fathom MUST NOT install plugins, run login commands, or read provider credential stores directly.
- **FR-011**: Existing static-credential contexts MUST behave identically (no regression).
- **FR-012**: Protocol additions MUST be additive with generated TS bindings and round-trip tests.
- **FR-013**: Behavior MUST hold on all Tier 1 platforms (Ubuntu 26.04, Windows 11, macOS 26), including `.exe`/`.cmd` resolution on Windows.

### Key Entities
See data-model.md: `CloudAuthInfo`, `CloudProvider`, `PluginError`, `ExecPolicy`, `CredentialCacheEntry`.

## Success Criteria
- **SC-001**: 100% of plugin failure classes above produce a typed error naming plugin+provider+context (tested with fake plugins).
- **SC-002**: With N fleet polls in one credential lifetime, plugin invocations == 1 per distinct identity.
- **SC-003**: Zero occurrences of a fixture token in logs/events/exports (grep-asserted test).
- **SC-004**: Static-credential contexts pass the existing suites unchanged.

## Assumptions
- Operator has already installed and authenticated the vendor CLI as for `kubectl`; Fathom is not a login flow.
- Real-cloud validation is manual (quickstart); CI uses fake plugin scripts.
- Out of scope: vendoring plugin logic, OIDC/IAM reimplementation, non-exec auth providers (legacy `auth-provider: gcp/azure`, removed upstream), Windows-only credential managers.
- P4 daemon and P2 federation are independent.
