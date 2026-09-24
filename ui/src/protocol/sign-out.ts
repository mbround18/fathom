/** Hosted sign-out (031): `POST /auth/logout` via the transport, then leave or show the gate. */
import { useStore } from '@/store'

import { getTransport } from './transport'

interface SignOutCapable {
  signOut(): Promise<string | null>
}

function canSignOut(t: unknown): t is SignOutCapable {
  return typeof t === 'object' && t !== null && 'signOut' in t && typeof t.signOut === 'function'
}

export async function signOutHosted(): Promise<void> {
  const transport = await getTransport()
  if (!canSignOut(transport)) return
  const endSessionUrl = await transport.signOut()
  if (endSessionUrl) {
    globalThis.location.assign(endSessionUrl)
    return
  }
  const { hosted } = useStore.getState()
  useStore.setState({ hosted: { ...hosted, sessionEnded: 'signedOut' } })
}
