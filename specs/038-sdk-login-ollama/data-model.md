# Data Model: SDK Login and Local Ollama Inference

- **Provider** (enum): `github | claude | openai | ollama`.
- **ProviderCredentialRef**: `{provider, account_label?, stored_in: "os_keychain" | "file_optin"}`. The secret itself lives only in the OS store under `fathom / ai-assist/<provider>`; never serialized.
- **ProviderStatus**: `{provider, state: NotConfigured | Connecting | Connected | NeedsReauth | Blocked{reason: Offline|StoreLocked|StoreUnavailable}, account_label?, last_checked_at}`.
- **LoginFlow** (transient, in-memory, single active): `{provider, kind: DeviceCode | ApiKey, user_code?, verification_uri?, expires_at?, interval_s?, phase: Pending | Polling | Succeeded | Denied | Expired | Failed{message}}`.
- **OllamaState**: `{endpoint, reachable, version?, installed_hint: Unknown|Found|NotFound, models: OllamaModel[], checked_at, error?}`.
- **OllamaModel**: `{name, size_bytes, family?, parameter_size?, modified_at}`.
- **AiAssistSettings** (persisted in `~/.fathom/config.json`, non-secret): `{active_provider?, ollama_endpoint, ollama_model?, offline_mode: bool, allow_lan_ollama: bool, file_store_optin: bool}`.
- Migration of 013 `AgentAuthMode`: `Anthropic{key}` -> provider `claude` + credential in keyring; `Ollama{endpoint}` -> `AiAssistSettings.ollama_endpoint`; `Disabled` -> `active_provider = None`.
- Invariants: at most one LoginFlow; `offline_mode` implies effective provider is `ollama` or none; secrets never in any persisted JSON except the opt-in 0600 file.
