# Implementation Plan: Keel Enroll, System Settings, admin tokens, OIDC roles, System workspace and UI polish

**Branch**: `main` (agent work lands on worktree branches, merged into `main`) | **Date**: 2026-10-28 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/060-keel-enroll-settings/spec.md`

## Summary

Give Keel's owners and admins a real place to bring hosts online and run the instance.

1. **Enroll dialog (US1)**:
   - an Enroll button and dialog on Hosts;
   - shows the one-time token, a snippet rendered by the server from an admin template, and
     Markdown instructions;
   - fixes the picker so owners see every workspace (R2, R3, R6).
2. **System Settings (US2, US3, US4)**:
   - the Enrollment tab (template and instructions, versioned and audited: R1);
   - the Tokens tab (enrolment tokens with revoke and roll, and every API token with admin revoke:
     R4, R5).
3. **OIDC roles (US5)**: instance roles from a mapped ID-token claim, re-applied on each sign-in,
   guarded by the last-owner rule (R7).
4. **System workspace (US7)**: an owner-only workspace that runs the infra Ansible against Keel's
   own host. Enforced in `authz::require`. Late results survive a Keel restart (R8, R9).
5. **Polish (US6)**: shared UI primitives, every section migrated, then the impeccable critique,
   polish and harden pass (R10).

## Technical Context

**Language/Version**: Rust stable (edition 2024) for keel-core, keel-protocol, keel-exec,
web-auth, keel-server and keel-runner. TypeScript, React 19 and Vite for `apps/keel/web`.

**Primary Dependencies**:
- Existing: `sqlx`, `ts-rs`, `radix-ui`, phosphor icons.
- New (approved): `react-markdown` and `remark-gfm`, in the web app only, lazy-loaded.

**Storage**: PostgreSQL, through migration `0010_settings_system.sql` (data-model.md). On the host,
the runner's undelivered spool.

**Testing**:
- `cargo test` for each crate touched;
- the keel-server database tests (throwaway PostgreSQL 18);
- Vitest, including the shared template vectors;
- Playwright, one spec per feature, plus screens at 360 and 1280 px.

**Target Platform**: The Keel server as a Linux container. The runner on Debian hosts. Evergreen
browsers.

**Project Type**: An existing web service, agent and web app (spec 055).

**Performance Goals**:
- The template preview updates on each keystroke, with no round trip.
- The Tokens tab lists 1,000 rows in under 1 s.
- Sign-in adds no measurable latency: the mapping is in-memory.

**Constraints**:
- Token values are never stored or shown twice; hashes only.
- Every mutation goes through preview and confirm, is audited, and fails closed.
- Step-up for save, revoke, roll and System changes.
- No raw HTML is rendered.
- Owner-only enforcement sits in one place.

**Scale/Scope**: One migration, 9 new commands, 4 new events, 3 changed events, 4 settings tabs,
7 shared UI primitives, and a pass over every section.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Assessment |
|---|---|
| I. Operator-first, explicit mutation | Pass. Every new mutation (settings save, revoke, roll, System configure and remove, admin token revoke) is a preview, then a confirm with step-up. Role changes from OIDC happen only at the person's own sign-in, and are audited with the reason `identity_provider`. |
| II. One contract per app | Pass. Every addition goes into `keel-protocol`, with regenerated bindings. The late-result replay reuses the existing `Finished` frame, with no new channel. The template renderer is authoritative on the server; the web twin is preview-only, and pinned to the same vectors. |
| III. Tested at the boundaries | Pass. The vectors are shared by Rust and TypeScript. Authz unit tests cover `owner_only`. Server DB tests cover settings, tokens, roles, the System workspace and late results. Runner tests cover the spool. There is one Playwright spec per feature. |
| IV. Density scales with width | Pass. The dialog and tabs are designed mobile-first. The polish pass screenshots every section at 360 and 1280 px. |
| V. Fast and quiet | Pass. The Markdown renderer and the settings pages load lazily. There is no polling: lists refresh on confirm events. |
| VI. User data is sacred | Pass. Enrolment rows are never deleted (revoke and roll keep history through `rolled_from`). Audit stays insert-only. System removal archives, and never deletes, the workspace. The last owner can't be demoted by the IdP. |

Post-design re-check: still passes. The one new dependency (react-markdown and remark-gfm) was
approved, and is confined to a lazy chunk. Nothing goes in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/060-keel-enroll-settings/
├── plan.md
├── research.md            # R1–R10
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── protocol.md        # Cmd/Event additions
│   ├── template-vectors.json
│   ├── config.md          # KEEL_OIDC_ROLE_*
│   ├── system-workspace.md
│   ├── runner-replay.md
│   └── web-ui.md
└── tasks.md               # /speckit-tasks
```

### Source code

```text
crates/keel-protocol/src/{cmd,event,…}.rs        # new Cmds/Events, changed events
crates/keel-core/src/enroll_template.rs          # validate + render (new)
crates/keel-core/src/authz.rs                    # Subject.system_workspace, Refusal::OwnerOnly
crates/web-auth/src/oidc.rs                      # dotted claim paths
crates/keel-exec/src/frames.rs                   # (unchanged shape; replay uses Finished)
apps/keel/server/migrations/0010_settings_system.sql
apps/keel/server/src/settings.rs                 # GetSettings, PreviewSettingsUpdate (new)
apps/keel/server/src/enrollments.rs              # list, revoke, roll (new)
apps/keel/server/src/system.rs                   # GetSystem, configure, remove (new)
apps/keel/server/src/{hosts,api,identity,signin,config,serve,dispatch,governance}.rs
apps/keel/server/src/{runner_listener,runs}.rs   # late Finished
apps/keel/runner/src/                            # undelivered spool + replay
apps/keel/web/src/ui/                            # shared primitives (new)
apps/keel/web/src/features/hosts/                # EnrollDialog, picker fix
apps/keel/web/src/features/settings/             # SettingsPage, tabs, template.ts, Markdown.tsx (new)
apps/keel/web/src/features/users/                # idp-managed rows
apps/keel/web/src/app/{nav.ts,routes.tsx,Shell.tsx}
apps/keel/web/tests/{unit,e2e}/
docs/guides/keel/*.md                            # enrolment template, OIDC roles, System workspace
```

**Structure decision**:
- The main session lands the contract first: protocol types, bindings, migration and the template
  module.
- Then parallel agents take disjoint areas:
  - server settings and tokens;
  - OIDC roles;
  - the System workspace and runner replay;
  - the web.
- The impeccable polish runs last, over the merged web.

## Complexity Tracking

No violations.
