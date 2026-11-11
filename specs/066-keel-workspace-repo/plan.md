# Implementation Plan: Keel workspaces, repository catalog and hosts

**Branch**: `066-keel-workspace-repo` | **Date**: 2026-11-11 | **Spec**: [spec.md](spec.md)

## Summary

Keel already has workspaces, bindings, sync, enrolment and inventories. This plan adds five
things on top of them:

- **One current workspace**, chosen from a switcher in the shell, with create.
- **A primary repository per workspace**, configured from Settings → Workspace.
- **A static catalog** of the primary repository, built on every sync: playbooks, roles,
  inventories, sops metadata, and job drafts derived from them.
- **Sops awareness** that never touches a value. Runners report their age public key, so Keel can
  tell which hosts can decrypt what.
- **Add host**: enrolment plus a pull request that edits the inventory with comments kept, through
  a forge port with Forgejo/Gitea and GitHub adapters.

## Technical Context

**Language/Version**:

- Rust (edition 2024, the toolchain in `rust-toolchain.toml`) for keel-core, keel-protocol,
  keel-exec, the new keel-forge, and the server and runner.
- TypeScript, React and Vite for `apps/keel/web`.
- Markdown for `docs/guides/keel`.

**Primary Dependencies**: existing.

- `serde_yaml_ng` or whichever YAML crate keel-core already uses (match `inventory.rs`).
- `regex` (check `cargo tree -p keel-core`; add it to keel-core when it is only present
  transitively, since it is already in the lockfile).
- `reqwest` with rustls for keel-forge (already in the lockfile, through keel-git's gix
  transport).
- **No new third-party crates beyond moving these into direct dependencies.**

**Storage**: PostgreSQL. Migration `0014_workspace_repo.sql` ([data-model.md](data-model.md)).

**Testing**:

- `cargo test -p keel-core catalog`, `cargo test -p keel-core inventory_edit`: pure, against
  in-memory trees, including a fixture copied from the operator's repository layout
  (`crates/keel-core/tests/fixtures/ansible-repo/`, synthetic content, no real keys).
- `cargo test -p keel-forge`: adapters against a local `axum` fake of the Forgejo and GitHub
  endpoints.
- `cargo test -p keel-server --test workspace_repo`, which is Postgres-backed. It covers:
  - primary set and clear, and the default project;
  - catalog stored on sync;
  - the decrypt view;
  - the run preview warning;
  - host add with the fake forge, and without it;
  - the SC-004 ciphertext grep over responses, the catalog row and audit rows.
- `cargo test -p keel-runner`: `Hello` carries `age_recipient` from the file.
- Vitest: the workspace store's resolution order, `visibleTabs`, catalog view helpers.
- Playwright: project `s066` (`tests/e2e/s066/`) against the fake backend, which gains
  `fake-catalog.ts` and `fake-forge.ts`. Also `polish-screens` at 360 and 1280 px for the
  switcher, the Repository page and the Add host dialog.

**Constraints**:

- Nothing in the catalog path executes repository content.
- No sops value is ever kept (R5).
- Forge writes only ever go to a new branch (R8).

## Constitution Check

- **Agnostic core and adapters**: the catalog, sops parsing, drafts and inventory edits are pure
  keel-core. Git reading stays in keel-git, and forge HTTP calls are a port in keel-core with
  adapters in keel-forge.
- **Tests first** for core logic: the catalog, the edits and the drafts are TDD.
- **Runtime feature flag**: `KEEL_FORGE_WRITES`.
- **OpenTelemetry**: spans `catalog.build`, `forge.open_change` and `forge.probe`, with attributes
  for counts only, never paths of secret files or values.

## Project Structure

```text
crates/keel-core/src/catalog/{mod,playbook,role,inventory,sops,draft}.rs   (new)
crates/keel-core/src/inventory_edit.rs                                      (new)
crates/keel-core/src/ports.rs                                               (+ Forge)
crates/keel-forge/                                                          (new crate)
crates/keel-protocol/src/{cmd,event,governance,host,binding,types}.rs       (+ types, cmds)
crates/keel-protocol/src/repository.rs                                      (new)
crates/keel-exec/src/frames.rs                                              (+ age_recipient)
apps/keel/runner/src/…                                                      (read recipient file)
apps/keel/server/migrations/0014_workspace_repo.sql                         (new)
apps/keel/server/src/repository.rs                                          (new: catalog, drafts, host add, forge)
apps/keel/server/src/{sync,governance,hosts,runs,runner_listener,config}.rs (hooks)
apps/keel/web/src/app/{workspace-store.ts,WorkspaceSwitcher.tsx,Shell.tsx,nav.ts,routes.tsx}
apps/keel/web/src/features/repository/                                       (new)
apps/keel/web/src/features/settings/WorkspaceTab.tsx                         (new)
apps/keel/web/src/features/hosts/AddHostDialog.tsx                           (new)
docs/guides/keel/{workspaces,repository}.md                                  (new) + edits
ansible: roles/sops writes /etc/sops/age.pub                                  (sibling repo)
```

## Phases and parallelism

1. **Foundation** (one agent, blocks the rest):
   - the migration;
   - the protocol types and commands, with server handlers stubbed to `not_implemented`;
   - the ts-rs regeneration;
   - the empty keel-core modules and the keel-forge crate skeleton;
   - the `s066` Playwright project.
2. **Parallel**, each with its own files:
   - **A**, core catalog and sops: keel-core `catalog/*` and its fixture.
   - **B**, core edits and forge: keel-core `inventory_edit.rs` and keel-forge.
   - **C**, web workspace: the store, the switcher, the page migration, the Settings Workspace
     tab, and the US1 and US2 server handlers in `governance.rs`.
3. **Integration** (after A and B):
   - **D**, the server: `repository.rs`, sync hook, run preview warning, host add, the runner
     `age_recipient`.
   - **E**, the web: Repository page, Add host dialog, host badges, fake backend.
4. **Docs and gate**: the guides, the CHANGELOG and the ansible `roles/sops` change. The main
   session then runs the full gate once.
