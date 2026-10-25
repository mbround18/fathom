# Implementation Plan: Depths root boundary and passkeys

**Branch**: `054-depths-root-boundary` | **Date**: 2026-10-25 | **Spec**: [spec.md](spec.md)

## Summary

This plan stops a compromised Depths from being root on the host in three ways. Research R1–R14
holds the decisions; the contracts and data model hold the shapes.

1. **Runtime guard (US1, US2).** A new service, `depths-guard`, stands between Depths and the
   existing socket proxy.
   - It is a small Rust proxy from its own scratch image, with no socket and no privileges.
   - It denies by default.
   - It judges container create, update, start and exec bodies, mounts, volumes and networks
     against the 051 rules. Those rules are shared through a new agnostic crate,
     `depths-guard-core`.
   - It walks bind sources for symbolic links, and protects itself, the socket proxy and their
     networks.
   - It logs to a host folder that only it can write.
   - Exceptions live only in a host-owned `guard.toml` (Q1 = A).
   - `depths-guard check` lists the containers that need exceptions before an upgrade.
2. **Helper image (US3).** The volume helper moves into a `FROM scratch` image that holds only its
   static binary. It keeps 053's run flags: no network, `--cap-drop ALL`, read-only,
   no-new-privileges, and limits.
   - The server and the helper share a compile-time build id. The server checks it in `Hello` and
     in the image label, and never falls back to its own image.
3. **Passkeys and step-up (US4, US5).**
   - `webauthn-rs` on the server, and `navigator.credentials` in the web client.
   - Admins and owners get the restricted view until their session is verified.
   - Every role must give a fresh passkey assertion for risky actions (Q2 = A), within a 5-minute
     window that can be changed. The assertion is carried by the existing `Confirm`, or by
     `OpenShell`.
   - Owners recover through a one-time code made on the host.

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19.

**Primary Dependencies**:

- The existing workspace: hyper 1, hyper-util, tokio, serde, rustix and toml.
- New:
  - `webauthn-rs` 0.5 (`openssl` vendored), in the server;
  - `webauthn-authenticator-rs` (soft passkey), as a server dev-dependency only;
  - `toml`, in the guard, if it is not already in the workspace.
- No new web dependency.

**Storage**:

- `<data_dir>/passkeys.json` and `<data_dir>/passkey-recovery.json`, written atomically in the
  `users.rs` pattern.
- The guard's JSON-lines log, in a host folder.
- `guard.toml`, read-only and host-owned.

**Testing**:

- A table-driven rule suite in `depths-guard-core`, covering every 051 escape code plus the forms
  in R3.
- A proxy test in `depths-guard` against an in-process fake upstream.
- Server tests with a soft passkey.
- Protocol round trips and binding drift checks.
- Playwright projects `us7-passkeys` (Chromium virtual authenticator) and `us8-guard`, against the
  fake backend.
- Opt-in Docker integration tests (`DEPTHS_IT_DOCKER=1`) for the guard and the helper image.

**Target Platform**: a Linux Docker host. The guard needs Linux 5.6 or later for `openat2`, the
same as the 053 helper. `nosymfollow` (5.10 or later) is optional and recommended.

**Project Type**: a web service (the Depths server and web UI), with a sidecar proxy service.

**Performance Goals**:

- The guard adds 5 ms or less at p95 to non-streaming requests on the dev box.
- Streams and upgrades are passed through unbuffered.
- The guard idles under 1% of a core and 32 MiB of RSS.
- A passkey confirmation takes under 10 s of interaction (SC-006).

**Constraints**:

- Fail closed everywhere.
- The guard never rewrites requests.
- No secrets, challenges or codes appear in the logs or the audit log, and no file contents appear
  in guard logs (Depths' "never log paths" rule is for the server; the guard logs only the setting
  it judged).
- Compose services use `restart: ${RESTART_POLICY:-no}`.

**Scale/Scope**:

- About 50 request kinds and about 30 rules.
- A handful of exceptions per host.
- Fewer than 20 people per Depths, each with up to 10 passkeys.

## Constitution Check

*GATE: before Phase 0, and again after Phase 1.*

| Principle | Check | Status |
|---|---|---|
| I. Explicit mutation | Passkey removal, recovery and every risky action go through preview and `Confirm`. Step-up is folded into the confirm step (FR-019). A guard refusal reaches the operator verbatim, as rule plus setting. Nothing is shown as changed until the runtime answers. | PASS |
| II. One contract | All the new commands and events are in `depths-protocol`, with ts-rs bindings and a version bump. The guard is a separate trust domain. Its two read-only endpoints are a documented contract (contracts/guard.md) that only the server consumes; the UI never talks to the guard. The rules live in the agnostic crate `depths-guard-core`, with the guard binary as the adapter. | PASS |
| III. Boundary tests | Round trips for every variant. Playwright us7 and us8, and the existing us1–us6 suites keep passing. A rule-table unit suite. The proxy is tested against a fake upstream. Each behaviour change has a failing-first test. | PASS |
| IV. Density | The passkey prompt sits inside the existing confirm dialog. The account page's passkey list and the stack page's exception list follow the existing mobile-first list patterns. | PASS |
| V. Fast and quiet | The guard adds no polling. Exceptions are fetched with the stack page, not on a timer. | PASS |
| VI. User data | Existing stacks and containers are not touched. An unreadable `passkeys.json` is moved aside, never deleted. Upgrade notes and `guard check` come before any change to steam. The constitution says socket access is root, so the docs keep saying so, and now explain what the guard narrows. | PASS |
| Platforms | Linux container host only, as before. The socket proxy stays the documented default, with the guard now in front of it. | PASS |

Re-check after Phase 1: no new violations. Confirm-step folding keeps principle I, and the guard
endpoints are read-only (II).

## Project Structure

### Documentation (this feature)

```text
specs/054-depths-root-boundary/
├── plan.md  research.md  data-model.md  quickstart.md
└── contracts/
    ├── guard.md      # topology, proxying, refusal response, guard endpoints, CLI
    ├── protocol.md   # Cmd/Event changes, gates, helper Hello
    └── config.md     # server env, guard config, compose env
```

### Source Code

```text
crates/depths-guard-core/                 # agnostic: no I/O
  src/lib.rs  endpoint.rs                 # classify(method, path) -> Endpoint
  src/body.rs                             # typed create/update/exec/network/volume bodies, unknown-field refusal
  src/rules.rs                            # judge(...) -> Verdict; reuses depths-core::security
  src/config.rs                           # GuardConfig + validation (TOML)
  src/exception.rs                        # matching and grants
crates/depths-core/src/security.rs        # expose the default-cap set; any helpers the guard needs
apps/depths/guard/                        # binary `depths-guard`
  src/main.rs  serve.rs  proxy.rs         # hyper server/client, upgrade passthrough
  src/lookup.rs                           # upstream inspect of containers/volumes; protected ids
  src/paths.rs                            # openat2 walk beneath roots (RESOLVE_NO_SYMLINKS)
  src/log.rs  check.rs  health.rs
  tests/proxy.rs  tests/docker.rs         # fake upstream; opt-in real daemon
apps/depths/server/src/passkeys.rs        # store, ceremonies, recovery codes
apps/depths/server/src/step_up.rs         # verification + step-up gates
apps/depths/server/src/{ops,ws,session_meta,config,main,audit,files}.rs   # wiring, Confirm/OpenShell assertion, CLI subcommand
crates/depths-runtime-cli/src/{lib,helper}.rs   # parse guard refusals; Hello.build check
crates/depths-volumes/src/op.rs           # Hello.build
apps/depths/volume-helper/src/main.rs     # send build id
crates/depths-protocol/src/{cmd,event,types}.rs
apps/depths/Dockerfile                    # targets: guard, helper (scratch); main image drops the helper
apps/depths/compose.yaml  .env.example  guard.toml.example
apps/depths/web/src/features/passkeys/    # codec, enrol gate, account list, confirm-step prompt
apps/depths/web/src/features/stacks/      # guard refusal display, exceptions list
apps/depths/web/tests/e2e/us7-passkeys/  us8-guard/
docs/guides/depths/guard.md  passkeys.md  # new; security.md, install.md, upgrade notes updated
```

**Structure Decision**:

- The rules go in a reusable agnostic crate (`crates/depths-guard-core`). The guard is a thin
  binary in `apps/depths/guard`, built into its own scratch image from the same Dockerfile.
- Passkeys stay in the Depths server. The shared `web-auth` crate stays provider-only, because
  passkeys here are Depths' second factor, not a general login method. They can move to
  `web-auth` later if another app needs them.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| A new service and image (the guard) | FR-001 needs the rules outside Depths' control | Checks inside Depths are bypassed by the very compromise this spec targets. An authz plugin needs host daemon config. |
| Guard in front of the socket proxy (two hops) | The guard holds no socket, so a guard bug is not root. The proven endpoint filter stays. | Guard holding the socket: one hop fewer, but its parser bugs would be root. |
| `depths-guard-core` crate | One rule set for the lint and the guard (no drift) | Restating the rules in proxy config would drift, and drift is a security bug. |
| OpenSSL (vendored) via webauthn-rs | Audited relying-party verification | Hand-rolled WebAuthn verification, or a young pure-Rust crate. |
| Second image (helper) | FR-011: the helper image holds no shell, no curl, no Docker CLI | Keeping the helper in the main image is today's state, which the spec removes. |
