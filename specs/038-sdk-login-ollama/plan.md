# Implementation Plan: SDK Login and Local Ollama Inference

**Branch**: `038-sdk-login-ollama` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

## Summary
Replace 013's plaintext single-key auth with a provider-login layer (GitHub device flow, Claude/OpenAI keys) backed by the OS credential store, add Ollama detection/model selection, and enforce an offline mode at one outbound transport chokepoint. Backend owns everything (Principle II); the UI speaks protocol v1 only.

## Technical Context
**Language**: Rust 2024 backend; TypeScript strict / React 19 in `ui/` (Tauri v2).
**New deps**: `keyring` (per-OS features), `secrecy`. Existing: `reqwest`, `tokio`, `serde`, `ts-rs`.
**Existing code touched**: `src/backend/agent/{credentials,ollama,anthropic,mod}.rs`, `src/backend/cmd/agent.rs`, `src/protocol/v1/{agent,cmd,event}.rs`, `src/backend/persistence.rs`, Settings -> Cluster Settings "Available Agents" (015) and `AuthSetup.tsx`.
**Testing**: cargo tests with fakes (device server, validators, Ollama stub, keyring double, recording transport); Vitest; Playwright.
**Platforms**: Ubuntu 26, Windows 11, macOS 26 (Tier 1). Linux CI needs the `keyring` mock store.
**Constraints**: no secret in logs/events/JSON; offline fails closed; probes <= 3 s; Fast and Quiet (no polling when the panel is closed).

## Constitution Check
- I Explicit mutation: no new mutation path. PASS.
- II One contract: all via versioned protocol v1 commands/events. PASS.
- III Boundary tests: contract, leak and offline tests planned. PASS.
- IV Density: settings panel responsive at phone width. PASS.
- V Fast/Quiet: probes on demand only; device-flow polling only while pending. PASS.
- VI User data sacred: non-destructive migration; credentials in the OS store. PASS.

## Project Structure
```
specs/038-sdk-login-ollama/{spec,plan,research,data-model,quickstart,tasks}.md, contracts/
src/backend/ai/            # new: mod, providers/{github,claude,openai}.rs, store.rs (keyring), http.rs (AiHttp offline guard), ollama_probe.rs, migrate.rs
src/backend/cmd/ai.rs      # command handlers
src/protocol/v1/ai.rs      # types + ts-rs
ui/src/features/ai-assist/ # ProviderLogin, OllamaPicker, OfflineToggle, status hook
```
`src/backend/agent/*` keeps conversation logic and obtains models/credentials via the new `ai` module.

## Phases
0 Research (done). 1 Contracts/data model (done). 2 Tasks (`tasks.md`).
Sequencing: protocol types -> store + migration -> AiHttp/offline -> Ollama probe -> providers -> UI -> integration with the 015 panel -> docs/MVP.md update (brochure screenshots last).
