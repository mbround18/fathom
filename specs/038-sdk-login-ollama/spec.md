# Feature Specification: SDK Login and Local Ollama Inference

**Feature Branch**: `038-sdk-login-ollama`
**Created**: 2026-09-24
**Status**: Draft
**Input**: MVP.md Tier A ("SDK-based login (GitHub / Claude / OpenAI)" and "Local LLM inference via Ollama for the MCP/AI-assist surface") and Tier E (per-operator auth modes, offline mode). Builds on `013-agent-sdk-ops` (assistant, auth modes) and `015-mcp-agent-exposure` (MCP surface, Ollama summary).

## Context and Assumptions (informed defaults, no open questions)

- 013 shipped BYOK Anthropic and a basic Ollama endpoint mode, with the key stored as plain JSON in `~/.fathom/credentials.json`. This feature replaces that storage with the OS credential store and adds provider login, Ollama detection/model pick, and an enforced offline mode.
- "SDK-based login" means each provider is reached through its official or public interface: GitHub via the OAuth device flow (public client, no embedded secret) to use GitHub Models; Claude and OpenAI via operator-supplied API keys validated against the provider (neither offers a third-party desktop OAuth flow today). The provider layer is an adapter trait so an OAuth flow can replace key entry later without UI change.
- The assistant remains read-only-by-default; agent-proposed mutations keep the 013 diff-preview + confirm gate (Principle I). Nothing here adds a mutation path.
- Supported Tier 1 platforms (Ubuntu 26, Windows 11, macOS 26) all provide an OS credential store (Secret Service, Credential Manager, Keychain).
- Ollama is assumed at `http://127.0.0.1:11434` by default; the endpoint is operator-editable and must be loopback unless the operator explicitly allows a LAN host.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sign in to a provider from settings (Priority: P1)

An operator opens Settings -> AI Assist, picks GitHub, Claude or OpenAI, and signs in. GitHub shows a device code and verification URL and completes when the operator authorizes in their browser. Claude and OpenAI accept an API key, which is validated with a cheap provider call before being saved. The panel then shows the connected account/provider, never the secret.

**Why this priority**: Without a working login there is no non-Ollama model access; this is the core of the feature.

**Independent Test**: With a fake provider endpoint, complete each login path and confirm the assistant status shows "connected" and a request is authorized; with invalid credentials confirm a specific error and nothing stored.

**Acceptance Scenarios**:
1. **Given** no provider configured, **When** the operator starts GitHub login, **Then** a user code, verification URL and expiry countdown are shown, and on authorization the provider shows connected with the account name.
2. **Given** the device code expires or is denied, **When** the flow ends, **Then** a specific message is shown and the operator can restart; no partial credential remains.
3. **Given** an API key for Claude or OpenAI, **When** submitted, **Then** it is validated against the provider; invalid keys are rejected verbatim with the provider's reason and never persisted.
4. **Given** a connected provider, **When** the operator clicks Sign out, **Then** the credential is deleted from the OS store, GitHub tokens are revoked best-effort, and the assistant returns to unconfigured.
5. **Given** a provider token later becomes invalid (401), **When** the assistant is used, **Then** it shows "re-authenticate" with a direct action rather than a generic failure.

---

### User Story 2 - Credentials stored securely and locally (Priority: P1)

Secrets live only in the OS credential store, never in `~/.fathom/*.json`, logs, protocol events, or the UI store. Existing 013 plaintext keys are migrated automatically and the plaintext copy removed.

**Independent Test**: After login, grep config files, logs and captured protocol traffic for the secret (none found); inspect the OS store entry exists; run migration on a fixture with a legacy plaintext key.

**Acceptance Scenarios**:
1. **Given** a saved credential, **When** any protocol event is emitted, **Then** it contains only provider id, account label, and status, never the token or key.
2. **Given** a legacy plaintext agent key in `credentials.json`, **When** the app starts, **Then** it is moved to the OS store, the JSON field removed, and the operator told once.
3. **Given** no usable OS credential store (e.g. headless Linux without Secret Service), **When** the operator saves a credential, **Then** Fathom refuses by default with instructions, and offers an explicit, labelled "store in a 0600 file" opt-in; it never silently falls back to plaintext.
4. **Given** the operator wants an explicit reveal, **When** they confirm, **Then** the existing 013 reveal semantics apply and the reveal is not logged.

---

### User Story 3 - Detect local Ollama and pick a model (Priority: P1)

Selecting Ollama probes the endpoint, reports version and reachability, lists installed models with size and capability hints, and lets the operator pick a default. If Ollama is not installed or not running, the panel says which and gives setup steps; if no model is installed it suggests a pull command.

**Independent Test**: Against a stub Ollama (`/api/version`, `/api/tags`) verify detected, not-running, and no-models states, and that the selected model persists and is used for a request.

**Acceptance Scenarios**:
1. **Given** Ollama running with models, **When** the operator opens the picker, **Then** models are listed and the selection is saved and used by the assistant and the MCP example chat summary.
2. **Given** Ollama not reachable, **When** probed, **Then** a "not running / not installed" state with instructions appears (fresh probe each time, no stale cache), never a fake model list.
3. **Given** the previously selected model was removed, **When** the assistant is used, **Then** the operator is prompted to pick again, not shown a raw 404.
4. **Given** a slow probe, **When** it exceeds 3 s, **Then** it times out with a clear state and does not block the UI.

---

### User Story 4 - Offline mode (Priority: P2)

A toggle "Offline mode" (or choosing Ollama as the sole mode) guarantees the AI-assist surface makes no non-loopback network calls: hosted providers are disabled and login/validation is blocked with an explanation, while Ollama continues to work.

**Independent Test**: With offline mode on, run an assistant turn against a stub Ollama and assert via a recording transport that zero requests target non-loopback hosts; attempt a hosted login and see it refused.

**Acceptance Scenarios**:
1. **Given** offline mode on, **When** any provider adapter would issue a request to a non-loopback host, **Then** it is refused at the transport layer (fail closed) and the UI states offline mode is why.
2. **Given** offline mode on and Ollama unreachable, **When** the operator asks a question, **Then** they get the Ollama setup state, not a fallback to a hosted provider.
3. **Given** offline mode is turned off, **Then** previously connected providers become usable again without re-login.

---

### User Story 5 - MCP/AI-assist surface reuses the same provider state (Priority: P3)

The Available Agents panel (015) and the assistant read one shared provider/auth summary (connected provider, Ollama state, selected model, offline flag) instead of separate detection.

**Acceptance Scenarios**:
1. **Given** provider status changes, **When** either surface is open, **Then** both update from the same event without a refresh.

### Edge Cases

- Two logins started concurrently: second is rejected while one is pending; pending device flow is cancellable.
- System clock skew or network loss during device-flow polling: honor `slow_down`/`interval`, surface network errors, allow retry.
- OS store locked (keychain prompt dismissed): show a specific "credential store locked" state, do not fall back.
- Ollama on non-default port or remote host: allowed only with explicit confirmation for non-loopback; offline mode rejects non-loopback.
- Very large model lists (100+): list is searchable and virtualized.
- Provider rate limiting (429): surfaced verbatim with retry-after.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Provide provider sign-in for GitHub (device flow), Claude (API key) and OpenAI (API key) behind a common provider adapter interface.
- **FR-002**: GitHub device flow MUST use a public client id with no embedded client secret and MUST respect the server-provided polling interval and expiry.
- **FR-003**: API keys MUST be validated against the provider before persistence; invalid keys are never stored.
- **FR-004**: All credentials MUST be stored in the OS credential store; secrets MUST NOT appear in JSON config, logs, `tracing` output, protocol events, or UI state.
- **FR-005**: Legacy plaintext agent credentials MUST be migrated to the OS store on first start and removed from disk; migration is idempotent and non-destructive on failure (plaintext retained only if the store write failed, with a warning).
- **FR-006**: With no OS credential store, saving MUST be refused unless the operator opts in to an explicitly labelled 0600 file; no silent plaintext fallback.
- **FR-007**: Operators MUST be able to sign out per provider, deleting the stored secret and revoking tokens where the provider supports it.
- **FR-008**: 401/expired credentials MUST surface a distinct "re-authenticate" state.
- **FR-009**: Ollama detection MUST probe on demand (no caching), report reachable/version/models, and distinguish not-installed/not-running/no-models where determinable.
- **FR-010**: Operators MUST be able to select a default Ollama model; selection persists in `~/.fathom/config.json` and is used by the assistant.
- **FR-011**: Offline mode MUST be enforced at the outbound transport layer for the AI-assist surface (non-loopback denied, fail closed), not merely hidden in the UI.
- **FR-012**: Provider selection remains per-operator; exactly one active provider at a time, others may stay connected.
- **FR-013**: The assistant and Available Agents panel MUST share one provider status event stream.
- **FR-014**: No new mutation path; agent-proposed actions retain the 013 confirmation gate.
- **FR-015**: All UI text and errors forward provider/Ollama errors verbatim with an actionable hint (Principle I).
- **FR-016**: The whole flow MUST work on Ubuntu 26, Windows 11 and macOS 26, and at phone-width density.

### Key Entities

See `data-model.md`: `Provider`, `ProviderCredentialRef`, `ProviderStatus`, `LoginFlow`, `OllamaState`, `OllamaModel`, `AiAssistSettings`.

## Testing Requirements

- Contract tests for new `BackendCmdV1`/`BackendEvent` variants and ts-rs bindings (Principle III).
- Backend tests with fake GitHub device-flow server, fake provider validators, stub Ollama, in-memory credential-store double, and a recording transport for offline enforcement.
- Secret-leak test: serialize every event and captured log during a login and assert the secret string is absent.
- Vitest for Settings -> AI Assist states; Playwright for the login and Ollama picker flows against stubs.

## Success Criteria *(mandatory)*

- **SC-001**: An operator completes provider login in under 60 s (excluding browser authorization time) on all Tier 1 platforms.
- **SC-002**: Zero occurrences of secrets in config files, logs, or protocol traffic across the leak test.
- **SC-003**: 100% of legacy plaintext keys migrated or explicitly reported on first launch.
- **SC-004**: Ollama detection resolves (success or a specific failure) within 3 s.
- **SC-005**: In offline mode, zero non-loopback requests are issued by the AI-assist surface (verified by recording transport).
- **SC-006**: Every failure state (denied, expired, locked store, no Ollama, no model, 401, 429) shows a specific message and next action.

## Out of Scope

Enterprise SSO/OIDC for AI providers (Tier B), hosted/cloud-ops mode, pulling Ollama models from inside Fathom, autonomous mutation, mobile.
