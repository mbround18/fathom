/**
 * Chromium's virtual authenticator, over CDP, standing in for the person's device so
 * `navigator.credentials` runs for real in a story. Copied from Keel's `tests/support/webauthn.ts`,
 * plus `seedPasskey` for a passkey registered before the story starts.
 */
import { generateKeyPairSync } from 'node:crypto'

import type { CDPSession, Page } from '@playwright/test'

import type { PasskeyInfo } from '../../src/protocol/generated'

export interface VirtualAuthenticator {
  cdp: CDPSession
  authenticatorId: string
}

/** Adds an authenticator that answers every prompt with a verified user. */
export async function virtualAuthenticator(page: Page, over: { isUserVerified?: boolean } = {}): Promise<VirtualAuthenticator> {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: over.isUserVerified ?? true,
      automaticPresenceSimulation: true,
    },
  })
  return { cdp, authenticatorId }
}

/**
 * Puts a credential for `localhost` on the authenticator and returns its `PasskeyInfo`, for
 * `backend.passkeys.enable({ items: [...] })`.
 */
export async function seedPasskey(auth: VirtualAuthenticator, name: string, created = '2026-10-01T09:00:00Z'): Promise<PasskeyInfo> {
  const raw = Buffer.from(`depths-${name}`)
  const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
  await auth.cdp.send('WebAuthn.addCredential', {
    authenticatorId: auth.authenticatorId,
    credential: {
      credentialId: raw.toString('base64'),
      isResidentCredential: true,
      rpId: 'localhost',
      privateKey: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
      userHandle: Buffer.from(`ada-${name}`).toString('base64'),
      signCount: 0,
    },
  })
  return { id: raw.toString('base64url'), name, created, last_used: null }
}

/** Makes the next prompts fail (user verification refused), as a person cancelling would. */
export async function cancelPrompts(auth: VirtualAuthenticator, cancel = true): Promise<void> {
  await auth.cdp.send('WebAuthn.setUserVerified', { authenticatorId: auth.authenticatorId, isUserVerified: !cancel })
}
