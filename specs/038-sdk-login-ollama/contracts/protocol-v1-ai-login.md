# Contract: protocol v1 additions (AI login and Ollama)

All are additive variants on `BackendCmdV1` / `BackendEvent` with ts-rs bindings. Existing 013 `Agent*` variants stay; `AgentSetAuthMode` becomes a compatibility shim mapped onto these.

## Commands
- `AiProviderLoginStart { provider }` -> device flow (github) or key-prompt state.
- `AiProviderSubmitApiKey { provider, key }` -- key is inbound only; validated then stored.
- `AiProviderLoginCancel {}`
- `AiProviderSignOut { provider }`
- `AiSetActiveProvider { provider? }`
- `AiSetOfflineMode { enabled }`
- `AiOllamaProbe { endpoint? }`
- `AiOllamaSelectModel { model }`
- `AiSetOllamaEndpoint { endpoint, allow_lan }`
- `AiEnableFileStore { confirm: true }`

## Events
- `AiProviderStatus { statuses: ProviderStatus[], active?, offline }` (single shared stream, also consumed by Available Agents)
- `AiLoginFlowUpdate { flow: LoginFlow }`
- `AiOllamaState { state: OllamaState, selected_model? }`
- `AiCredentialMigration { outcome: Migrated | Failed{message} | NothingToMigrate }`
- `AiError { code, message_verbatim, hint }`

## Guarantees
- No event or command Debug output contains secrets; `AiProviderSubmitApiKey` redacts in `Debug`/tracing.
- Offline denial returns `AiError{code:"offline_blocked"}` with no network I/O.
- Every command replies with an event (no silent success/failure).
