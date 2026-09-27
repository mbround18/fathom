import { useMemo } from 'react'

import { useStore } from '@/store'
import { setupInputsOf } from '@/store/setup'

import { setupStateOf, type SetupState } from './setup-state'

/** The app-wide setup state (043), or `undefined` when the normal shell renders. */
export function useSetupState(): SetupState | undefined {
  const session = useStore((s) => s.session)
  const hosted = useStore((s) => s.hosted)
  const setup = useStore((s) => s.setup)
  return useMemo(
    () => setupStateOf(setupInputsOf({ session, hosted, setup })),
    [session, hosted, setup],
  )
}
