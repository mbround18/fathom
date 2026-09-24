# Tasks: SDK Login and Local Ollama Inference

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/protocol-v1-ai-login.md. Tests included (Principle III).
Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup
- [ ] T001 Add `keyring` and `secrecy` to `Cargo.toml` with per-OS features; confirm `cargo check` on Linux.
- [ ] T002 [P] Scaffold `src/backend/ai/` module tree and `ui/src/features/ai-assist/`.

## Phase 2: Foundational (blocks all stories)
- [ ] T003 Define protocol types in `src/protocol/v1/ai.rs` (Provider, ProviderStatus, LoginFlow, OllamaState/Model, AiAssistSettings) with ts-rs export.
- [ ] T004 Add `Ai*` commands/events to `src/protocol/v1/cmd.rs` and `event.rs`; roundtrip serde tests.
- [ ] T005 [P] Implement `AiAssistSettings` persistence in `src/backend/persistence.rs` (`~/.fathom/config.json`), serde defaults for old files.
- [ ] T006 Implement `AiHttp` wrapper in `src/backend/ai/http.rs` (single client, offline flag, resolve-then-check loopback, fail closed) and a recording-transport test double.
- [ ] T007 Define the `ProviderAdapter` trait and shared status broadcaster in `src/backend/ai/mod.rs`.

## Phase 3: US2 Secure storage (P1)
- [ ] T008 [P] [US2] Tests: keyring double roundtrip, locked/unavailable store, no-fallback default, opt-in 0600 file.
- [ ] T009 [US2] Implement `store.rs` (keyring wrapper, `SecretString`, errors mapped to StoreLocked/StoreUnavailable).
- [ ] T010 [US2] Implement `migrate.rs` (legacy plaintext -> keyring, verify, atomic removal, `AiCredentialMigration` event); wire into startup.
- [ ] T011 [US2] Redacting `Debug` for the inbound key command; secret-leak test over events and captured logs.
- [ ] T012 [US2] Shim `AgentSetAuthMode`/`AgentRevealKey` onto the new store, preserving 013 reveal semantics.

## Phase 4: US1 Provider login (P1)
- [ ] T013 [P] [US1] Tests with fake GitHub device server (pending, slow_down, denied, expired, success) and fake Anthropic/OpenAI validators (valid, 401, 429).
- [ ] T014 [US1] Implement the GitHub device-flow adapter with a cancellable polling task honoring interval/expiry.
- [ ] T015 [P] [US1] Implement Claude and OpenAI key-validation adapters.
- [ ] T016 [US1] Command handlers in `src/backend/cmd/ai.rs`: login start/submit/cancel, sign out (revoke best-effort), set active provider; single-flow guard.
- [ ] T017 [US1] NeedsReauth handling: map 401 from assistant turns to status + event.
- [ ] T018 [US1] Route `src/backend/agent/anthropic.rs` and a new OpenAI/GitHub Models client through `AiHttp` and the store.

## Phase 5: US3 Ollama detect and model pick (P1)
- [ ] T019 [P] [US3] Tests with stub Ollama: detected, not running, no models, timeout, removed selected model.
- [ ] T020 [US3] Implement `ollama_probe.rs` (`/api/version`, `/api/tags`, 3 s timeout, PATH scan hint, no cache).
- [ ] T021 [US3] Handlers `AiOllamaProbe`, `AiOllamaSelectModel`, `AiSetOllamaEndpoint` (LAN confirmation); persist selection; use in `agent/ollama.rs`.

## Phase 6: US4 Offline mode (P2)
- [ ] T022 [P] [US4] Tests: recording transport asserts zero non-loopback requests across a full assistant turn; hosted login refused; toggling off restores providers.
- [ ] T023 [US4] Implement `AiSetOfflineMode`, effective-provider resolution, `offline_blocked` errors; remove any direct `reqwest::Client` use in agent code (grep-gate test).

## Phase 7: UI
- [ ] T024 [P] [US1] `ProviderLogin` component (device code view with countdown, key entry, sign out) with Vitest tests.
- [ ] T025 [P] [US3] `OllamaPicker` (searchable, virtualized, state panels) with Vitest tests.
- [ ] T026 [US4] `OfflineToggle` and status hook consuming `AiProviderStatus`; Settings -> AI Assist page, phone-width layout.
- [ ] T027 [US5] Replace the Available Agents duplicated auth summary with the shared status event; retire `AuthSetup.tsx` detection duplication.
- [ ] T028 Playwright flows: GitHub login (stub), invalid key, Ollama pick, offline.

## Phase 8: Polish
- [ ] T029 [P] Update `docs/MVP.md` Tier A/E statuses and `docs/mcp/MCP_ARCHITECTURE.md` provider section.
- [ ] T030 Run quickstart.md end to end on Ubuntu, Windows, macOS; `cargo clippy`, `cargo test`, `pnpm test`.
- [ ] T031 Brochure screenshots of new screens (final step of the batch).

## Dependencies
Phase 2 blocks all. US2 (Phase 3) precedes US1 (needs the store). US3 and US4 depend on T006 and can run in parallel with US1 after Phase 3. UI (Phase 7) after protocol (T004). T031 last.
