# Research: SDK Login and Local Ollama Inference

**R1 Credential storage.** Decision: `keyring` crate (v3) -> Secret Service (Linux), Keychain (macOS), Credential Manager (Windows). Service `fathom`, account `ai-assist/<provider>`. Alternatives: encrypted file with derived key (key management just moves the problem), plain 0600 file (current 013 behavior; kept only as explicit opt-in per FR-006). Non-goal: inventing crypto.

**R2 GitHub login.** Decision: OAuth device flow (RFC 8628) against `github.com/login/device/code` and `/login/oauth/access_token` with a public OAuth app client id; scope minimal (models access). No client secret is needed or shipped. Token used against the GitHub Models inference API. Alternative: PAT paste (kept as fallback field) and loopback redirect flow (needs secret handling, rejected).

**R3 Claude / OpenAI.** Neither provides third-party desktop OAuth; decision: API-key entry validated via a cheap authenticated call (Anthropic `GET /v1/models`, OpenAI `GET /v1/models`). Implemented over the already-present `reqwest` (matches 013 R1: no heavy SDK crate) behind a `ProviderAdapter` trait so an official SDK/OAuth can be swapped in. Reusing Claude Code CLI login rejected: its credentials are not a supported interface.

**R4 Ollama.** Decision: probe `GET /api/version` (reachable, version) and `GET /api/tags` (models, size, family) with 3 s timeout; not-installed vs not-running is best-effort via `PATH` scan for `ollama` (reusing the 015 fresh-scan approach). Chat continues to use the existing `src/backend/agent/ollama.rs`. Default endpoint `127.0.0.1:11434`.

**R5 Offline enforcement.** Decision: all AI-assist HTTP goes through a single `AiHttp` wrapper holding an `offline` flag and an allow policy (loopback only when offline); provider adapters cannot obtain a raw client. Fail closed on DNS results that are non-loopback (resolve then check IP, defeating look-alike hostnames). Alternative: UI-only gating, rejected as unenforceable.

**R6 Migration.** Decision: on startup, read the legacy `agent` key from `~/.fathom/credentials.json`, write to keyring, verify read-back, then remove the field atomically (tmp+rename). On failure keep legacy and emit a warning event.

**R7 Secret hygiene.** Use `secrecy::SecretString` in backend; protocol types carry no secret fields except the inbound `AiProviderSubmitApiKey` command (never echoed, redacted `Debug`).
