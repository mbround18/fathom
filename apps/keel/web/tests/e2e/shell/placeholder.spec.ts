/** Placeholder for the `shell` project (spec 055 T010): only opens the shell. */
import { test } from '@playwright/test'

import { openShell } from '../../support/shell'

test('shell: the shell opens', async ({ page }) => {
  await openShell(page)
})
