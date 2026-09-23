# Tasks: Container Image Inspection

**Input**: plan.md, research.md, data-model.md, contracts/image-inspection.md, spec.md

## Phase 0: Setup

- [ ] T001 Add `oci-client = "0.18"` to `Cargo.toml` (+ any needed dev-dependency for HTTP mocking,
  e.g. `wiremock`, under `[dev-dependencies]`). Commit dedicated to this dependency addition.

## Phase 1: Protocol (shared prerequisite for US1/US2/US3)

- [ ] T002 [P] Define `ImageRef`, `ImageLayer`, `BaseImageGuess`, `ImageInspection`,
  `ImageInspectErrorKind`/`ImageInspectError` in `src/protocol/v1/images.rs`.
- [ ] T003 Add `BackendCmdV1::InspectContainerImage { namespace, pod, container }` in
  `src/protocol/v1/cmd.rs`.
- [ ] T004 Add `ResourceReadEvent::ContainerImageInspected { namespace, pod, container, inspection }`
  and `ContainerImageInspectFailed { namespace, pod, container, error }` in `src/protocol/v1/event.rs`.
- [ ] T005 Wire new types into `src/protocol/v1/mod.rs` / `src/protocol/mod.rs` re-exports; regen
  TS bindings (`ui/src/protocol/types.ts`) via the project's existing ts-rs generation step.
- [ ] T006 [P] Protocol round-trip serde tests for all new types in `tests/protocol_serde.rs`.

## Phase 2: User Story 1 — Inspect a running container's image (P1)

**Goal**: On-demand manifest+config fetch for a public image, on-demand only, session-cached by digest.

- [ ] T010 [US1] `src/backend/images.rs`: image reference resolution (default registry/tag rules,
  matches Edge Cases in spec.md).
- [ ] T011 [US1] `src/backend/images.rs`: anonymous manifest fetch + manifest-list platform
  resolution (R3) via `oci-client`.
- [ ] T012 [US1] `src/backend/images.rs`: config blob fetch, layer/history assembly into
  `ImageInspection` (total size = sum(layer sizes) + config blob size).
- [ ] T013 [US1] `src/backend/state.rs`: `image_inspect_cache: HashMap<String, ImageInspection>`;
  cache-hit short-circuit before any network call.
- [ ] T014 [US1] Dispatch `InspectContainerImage` in `src/backend/mod.rs` -> `images::inspect_container_image`,
  emitting the two new events; never fetch on Pod detail read (FR-003).
- [ ] T015 [US1] Unit tests: manifest/config parsing + layer/history assembly against fixture
  JSON bytes (no live registry) in `src/backend/images.rs` `#[cfg(test)]`.
- [ ] T016 [US1] Unit test: cache short-circuit (second call for same digest makes 0 additional
  mock-HTTP requests).
- [ ] T017 [US1][P] `ui/src/store`: `imageInspections` slice keyed by `ns/pod/container`, updated
  from the two new events.
- [ ] T018 [US1][P] `ui/src/components/panels/ImageInspectPanel.tsx`: container picker (if >1),
  on-select/on-mount dispatch of `InspectContainerImage` (only when tab active — not eager),
  render registry/repo/digest/size/layers/history. All new testids prefixed `image-inspect-`.
- [ ] T019 [US1] Wire `image` tab into `ui/src/components/Inspector.tsx` for `target.kind === 'Pod'`.
- [ ] T020 [US1][P] Vitest component test for `ImageInspectPanel` (loading/success/no-eager-fetch
  behavior) in `ImageInspectPanel.test.tsx`.
- [ ] T021 [US1] Playwright e2e `ui/tests/e2e/us1-image-inspection-public.spec.ts`: scripted fake
  backend, asserts tab shows data, switching containers shows per-container data, re-opening a
  cached tab does not re-send the command.

**Checkpoint**: US1 independently testable/demoable.

## Phase 3: User Story 2 — Private registry via imagePullSecrets (P1)

- [ ] T030 [US2] `src/backend/images.rs`: read `spec.imagePullSecrets` off the live Pod;
  `Api<Secret>::namespaced` read + `.dockerconfigjson` parse (reusing helm.rs's Secret-read
  pattern).
- [ ] T031 [US2] `src/backend/images.rs`: anonymous-then-secret-fallback auth flow — try
  anonymous, on 401/auth-challenge iterate secrets, retry with `RegistryAuth::Basic`.
- [ ] T032 [US2] `src/backend/images.rs`: map failure modes to `ImageInspectErrorKind`
  (`RegistryUnreachable`, `ImageNotFound`, `AuthenticationFailed`, `AuthMethodUnsupported`, `Other`).
- [ ] T033 [US2] Unit tests: auth-fallback state machine against a mock HTTP server (`wiremock`)
  returning 401 + WWW-Authenticate, secret-not-matching case, OIDC/device-code-shaped challenge
  → `AuthMethodUnsupported`.
- [ ] T034 [US2][P] `ImageInspectPanel.tsx`: render each distinguishable error state (FR-009),
  each with its own `data-testid="image-inspect-error-<kind>"`.
- [ ] T035 [US2] Playwright e2e `ui/tests/e2e/us1-image-inspection-private-auth.spec.ts`: success
  via secret fallback, and each error state rendered distinctly.

**Checkpoint**: US1+US2 independently testable/demoable.

## Phase 4: User Story 3 — Best-effort base image (P3)

- [ ] T040 [US3] `src/backend/images.rs`: built-in base-image digest table + matching heuristic
  (R5).
- [ ] T041 [US3] Unit tests: match found / not found cases.
- [ ] T042 [US3][P] `ImageInspectPanel.tsx`: render best-effort label / "not determined" state,
  `data-testid="image-inspect-base-image"`.
- [ ] T043 [US3] Playwright e2e (folded into `us1-image-inspection-public.spec.ts` or its own
  `us1-image-inspection-base-image.spec.ts`): asserts best-effort label visible and distinctly
  labeled as such.

## Phase 5: Polish & verification

- [ ] T050 `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test` clean.
- [ ] T051 `pnpm -C ui typecheck`, `pnpm -C ui lint`, `pnpm -C ui test` clean.
- [ ] T052 Relevant Playwright e2e suite green.
- [ ] T053 Update `docs/MVP.md` §9a status row to shipped (mirrors 006-helm-mutate pattern).

## Dependencies

- Phase 0 → Phase 1 → Phase 2 (US1) → Phase 3 (US2) → Phase 4 (US3) → Phase 5.
- US2 builds directly on US1's fetch path (adds auth fallback); US3 builds on US1's layer data.
- Frontend tasks in each phase depend on that phase's protocol/backend tasks landing first.
