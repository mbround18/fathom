---
description: "Task list for spec 066, Keel workspaces, repository catalog and hosts"
---

# Tasks: Keel workspaces, repository catalog and hosts

**Paths**:

- `server/` means `apps/keel/server/`.
- `web/` means `apps/keel/web/`.
- `core/` means `crates/keel-core/src/`.
- `proto/` means `crates/keel-protocol/src/`.
- `forge/` means `crates/keel-forge/`.

**Gates for agents**:

- Run targeted checks only, one at a time, logging to files, with `CARGO_BUILD_JOBS=4`.
- Never run `scripts/ci.sh`, `paws`, `dagger` or the full Playwright suite.
- Write Playwright as `--project=s066`.

## Phase 1: Foundation

- [X] T001 Write `server/migrations/0014_workspace_repo.sql` per data-model.md. Confirm the
  credentials check constraint's name and UUID generation against earlier migrations.
- [X] T002 Add the protocol types and commands from contracts/protocol.md:
  - put the new types in `proto/repository.rs`;
  - extend `WorkspaceInfo`, `HostInfo`, `CredentialKind` and `Preview`;
  - extend action names and `is_mutating`;
  - extend the TS export list;
  - regenerate `web/src/protocol/generated`.
- [X] T003 Fill the new fields in the server, as `None`, empty or the current value:
  - `WorkspaceInfo.primary_binding` from the column;
  - `HostInfo.age_recipient` from the column;
  - `inventory_status: unknown`;
  - `Preview.warnings: []`.

  Route the new commands to handlers that refuse with `not_implemented`.
- [X] T004 Create the empty `core/catalog/` modules, `core/inventory_edit.rs`, the `Forge` port in
  `core/ports.rs`, and the `crates/keel-forge` skeleton. They should compile, with no logic yet.
- [X] T005 Add Playwright project `s066` and create `web/tests/e2e/s066/`. Add
  `server/tests/workspace_repo.rs` as an empty harness on `tests/common`.
- [X] T006 `PreviewWorkspaceCreate` also inserts the `default` project, in the same transaction
  and audit target. Update `tests/governance.rs`.

## Phase 2A: Catalog and sops (keel-core)

- [X] T010 Build fixture `crates/keel-core/tests/fixtures/ansible-repo/`, modelled on the
  operator's layout. It should include:
  - playbooks with imports;
  - roles with meta dependencies and `include_role`;
  - an inventory with child groups and comments;
  - `.sops.yaml` with two rules;
  - two sops files with synthetic age keys and ciphertext;
  - a `.keel/jobs.yaml`.
- [X] T011 Write tests first for playbook detection, role reach (literal, templated, collection,
  depth limit) and imports (R4).
- [X] T012 Implement `catalog/playbook.rs` and `catalog/role.rs`.
- [X] T013 Write tests first, then implement `catalog/inventory.rs`. It covers YAML (reusing
  `inventory.rs`), INI, dir and dynamic.
- [X] T014 Write tests first, then implement `catalog/sops.rs`:
  - rules;
  - metadata;
  - drift;
  - values dropped. The test asserts that the serialised catalog contains no fixture ciphertext.
- [X] T015 Write tests first, then implement `catalog/mod.rs` `build`:
  - limits and skipped counts;
  - problems;
  - jobs linkage;
  - `read_by` attribution (R5).
- [X] T016 Write tests first, then implement `catalog/draft.rs` (R6). Include a test that the
  drafts for the fixture's declared playbooks equal their declared `paths`.

## Phase 2B: Edits and forge

- [X] T020 Write tests first for `inventory_edit::add_host`, using the operator's real
  `inventory/hosts.yml` shape. Cover:
  - the existing group;
  - a child group;
  - a missing `hosts:`;
  - sort position;
  - CRLF;
  - `host_exists`;
  - flow style refused;
  - the re-parse check.
- [X] T021 Implement `add_host`, `add_sops_recipient` and `add_job`, each with tests.
- [X] T022 Implement `forge/` with the Forgejo/Gitea and GitHub adapters (R8). Test them against
  an axum fake that records requests. Cover:
  - the `sha` precondition;
  - a refusal mapping to `forge_refused`;
  - `probe` reading `permissions.push`.

## Phase 2C: Workspace UX (Story 1, Story 2)

- [X] T030 Write tests first, then implement `web/src/app/workspace-store.ts`:
  - resolution order (R1);
  - the address kept in sync;
  - the archived or vanished fallback.
- [X] T031 Add `WorkspaceSwitcher.tsx` to the shell's top bar, with the create dialog going
  through `ChangeDialog`.
- [X] T032 Move Runs, Reviews, Bindings, Receivers, Inventory and Hosts to `useCurrentWorkspace()`.
  Delete `GrantWorkspacePicker`.
- [X] T033 Server: add `PreviewWorkspacePrimary`, `PreviewWorkspaceRepository` (create the binding
  on `default`, set primary, then trigger a sync) and the forge connection commands, with
  `ProbeForge` through keel-forge. Add tests in `tests/workspace_repo.rs`.
- [X] T034 Add the Settings Workspace tab and update `tabs.ts` `visibleTabs` (R10). Make the nav's
  Settings entry visible to workspace admins.
- [X] T035 Playwright `s066/workspace-switcher.spec.ts` and `s066/settings-workspace.spec.ts`.

## Phase 3D: Server integration (Story 3, Story 4, Story 5)

- [X] T040 The sync hook: after a successful sync of a primary binding, build the catalog on the
  blocking pool through a `RepoFiles` impl over `Mirror`, and upsert `binding_catalogs`. On
  failure, mark it stale and keep the old document.
- [X] T041 `GetCatalog`, with per-host decrypt state, and `DraftJob`.
- [X] T042 `PreviewRunStart` warnings (FR-018).
- [X] T043 The runner reports `age_recipient` (frames, runner config `age_recipient_file`). The
  server stores it in the `runner_listener` hello handler.
- [X] T044 `PreviewHostAdd`. Its confirm issues the enrolment token through the existing path, and
  computes the edit against the inventory file at the head. Then it calls the forge, or returns
  `Manual` or `Failed`. Also add `PreviewSopsRecipient` and `PreviewJobPropose`. All three are
  audited.
- [X] T045 Derive `HostInfo.inventory_status` from the primary catalog.
- [X] T046 Add the SC-004 test, which greps every response, the catalog row and the audit rows for
  fixture ciphertext.

## Phase 3E: Web integration

- [X] T050 Add `fake-catalog.ts` and `fake-forge.ts` to the fake backend.
- [X] T051 Add `features/repository/`, the page with five lazily loaded tabs and detail panes, and
  a nav entry.
- [X] T052 Add the Draft job dialog, with Copy and Propose as pull request.
- [X] T053 Add `AddHostDialog.tsx`, the host inventory status badge, the age key on the host
  detail, and Propose sops recipient.
- [X] T054 Playwright `s066/repository.spec.ts`, `s066/secrets.spec.ts` and
  `s066/add-host.spec.ts`. Add the polish screens.

## Phase 4: Docs and gate

- [ ] T060 Write the guides `workspaces.md` and `repository.md`. Update `getting-started.md`,
  `hosts.md`, `bindings.md` and the glossary, and add the CHANGELOG entry.
- [ ] T061 In the ansible repo, `roles/sops` writes `/etc/sops/age.pub` (0644). Pin it
  with molecule or ansible-lint, per that repo's CONTRIBUTING.
- [ ] T062 The main session runs `scripts/ci.sh` once on the combined branch, then
  `scripts/hygiene.sh`.
