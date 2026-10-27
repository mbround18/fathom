/**
 * Chromium's virtual authenticator, over CDP, standing in for the person's device so
 * `navigator.credentials` runs for real in a story.
 */
import type { Page } from '@playwright/test'

export async function virtualAuthenticator(page: Page): Promise<void> {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  })
}
