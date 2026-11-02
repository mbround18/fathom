import type { Page } from '@playwright/test'

/**
 * Waits for every running CSS animation on the page to finish: a dialog's or a popover's fade and
 * zoom-in. axe measures colour contrast on what is painted, so a scan mid-fade reads a half-opaque
 * button as low contrast.
 */
export async function settled(page: Page): Promise<void> {
  await page.evaluate(() => Promise.all(document.getAnimations().map((a) => a.finished.catch(() => undefined))))
}
