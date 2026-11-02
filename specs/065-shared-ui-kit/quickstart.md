# Quickstart: validating the shared UI kit

Run these from the repository root. While agents are active, prefix any cargo build with
`CARGO_BUILD_JOBS=4`. Run one Playwright suite at a time.

## 1. The kit on its own

```sh
pnpm --filter @companion/ui typecheck
pnpm --filter @companion/ui test                          # unit tests
pnpm --filter @companion/ui e2e                           # every piece: behaviour and axe, in light and dark, at 360 and 1280 px
pnpm --filter @companion/ui e2e tests/e2e/load-failed.spec.ts   # one piece alone
```

Expected: everything passes, with zero axe violations. Then list the copies:

```sh
grep -rn "function EmptyState\|function LoadFailed" apps ui/src
```

Expected: no output.

## 2. The copy check

```sh
pnpm -w run lint:packages
```

Expected: it passes.

To prove the check works:

1. Add `export function EmptyState() { return null }` to any file in `apps/depths/web/src`.
2. Run the check again. It must fail and name `@companion/ui/kit`.
3. Revert the file.

## 3. Keel: nothing visible changed (US1, SC-003)

```sh
KEEL_E2E_PORT=4263 pnpm --filter @companion/keel-web e2e --project s065   # screenshot goldens taken before the move
pnpm --filter @companion/keel-web test
pnpm --filter @companion/keel-web e2e
```

Expected:

- `s065` passes with no pixel differences.
- `git diff 562900be -- apps/keel/web/tests` shows changes to import paths only, plus the new
  `s065` files.

## 4. Depths (US2)

```sh
pnpm --filter @companion/depths-web test
pnpm --filter @companion/depths-web e2e
```

Expected:

- The new specs pass:
  - every list that fails to load shows "Failed to load {what}." with the reason and Retry;
  - a page that throws shows "Something went wrong showing this page." with Reload, while the
    navigation still works.
- The change notes in `CHANGELOG.md` list Depths' visible changes.

## 5. Fathom (US3)

```sh
pnpm --filter fathom-ui test
pnpm --filter fathom-ui exec playwright test tests/e2e/perf-10k.spec.ts   # constitution V still holds
pnpm --filter fathom-ui e2e
```

Expected:

- In a pane that fails to load, only that pane shows the failed state.
- In a pane that throws, only that pane shows the crash state, and the other panes keep working.
- Sorting, selection and row actions work as before.
- Bulk delete still asks for the typed confirmation.

## 6. Gate

`scripts/ci.sh all`, once, in the main session, after every branch is merged.
