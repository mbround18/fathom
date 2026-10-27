/** Placeholder for the `us6` project (spec 055 T010): only opens the shell. */
import { test } from '@playwright/test'

import { openShell } from '../../support/shell'

test('us6: the shell opens', async ({ page }) => {
  await openShell(page)
})
