# Research: Observability Settings

## R1. Storage is not actually per context today
- **Finding**: `IntegrationConfig` in `src/backend/integrations.rs` is a single record read from `~/.fathom/integrations.json`; `load(context)` ignores its argument, so saving Prometheus for one context is visible from every context.
- **Decision**: add a per-context map (`by_context: { <context>: { prometheus, grafana } }`) alongside the legacy top-level fields. On load, a legacy record whose `context` matches is treated as that context's settings (lazy migration on next save). The original file is preserved, and unreadable files fall back to defaults without deletion.
- **Rationale**: Constitution VI (additive, no manual steps); spec FR-009 requires isolation.
- **Alternatives**: overwrite format (rejected: breaks downgrade); one file per context (rejected: more churn).

## R2. Grafana API key storage
- **Decision**: keyring entry, service `fathom.grafana`, user `<context>`, via a small `SecretStore` trait with keyring and in-memory implementations, mirroring the pattern in `backend/oidc/store.rs` and `backend/ai/store.rs`. `GrafanaConfig.api_key` is no longer serialized (kept as an ignored legacy field so old files load; a legacy plaintext value is moved into the keyring on first read and stripped on next save).
- **Rationale**: FR-002, Constitution VI, testability without a real keyring.
- **Alternatives**: encrypted file (rejected: key management, duplicates keyring work).

## R3. Read path shape
- **Decision**: command `GetObservabilitySettings {}` (context taken from the session), event `ObservabilitySettings { context, prometheus: Option<{namespace,service,port}>, grafana: Option<{base_url, key_set}> }`. Also emitted after `ConfigurePrometheus`, `ConfigureGrafana` and `ClearGrafanaKey` so forms refresh without a second request.
- **Rationale**: one round trip; the secret has no field in the type, so it cannot leak by construction.

## R4. Blank key on save and clearing
- **Decision**: `ConfigureGrafana` with an empty `api_key` keeps the stored key; new command `ClearGrafanaKey {}` removes it. The UI never prefills the key input.

## R5. Keyring unavailable
- **Decision**: write the URL, fail the key write, emit an error notification, and report `key_set: false`, so the UI does not claim fully configured (FR-010).

## R6. Gating and mutation classification
- **Decision**: `GetObservabilitySettings` is read-only (allowed for read-only viewers); `ConfigureGrafana` and `ClearGrafanaKey` join the existing mutation and section mapping (`section_keys::integrations_*`) so capability gating is unchanged.
