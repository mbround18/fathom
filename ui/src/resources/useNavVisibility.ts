/**
 * 043: the store-backed `NavVisibilityContext` the navigator and the command palette share, so
 * Velero (hidden until detected, FR-036) and Local clusters (desktop only) agree everywhere.
 */
import { useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'

import type { SectionKey } from '@/protocol/types'
import { useStore } from '@/store'

import { integrationInstalled } from './integrationStatus'
import { NAV_MODEL, type NavVisibilityContext } from './nav'

const INTEGRATION_KEYS: readonly SectionKey[] = NAV_MODEL.flatMap((g) =>
  g.entries.flatMap((e) => (e.integration ? [e.integration] : [])),
)

/** Detection per integration section for the active context (`undefined` = not scanned yet). */
export function useInstalledIntegrations(): Record<string, boolean | undefined> {
  return useStore(
    useShallow((s) =>
      Object.fromEntries(INTEGRATION_KEYS.map((k) => [k, integrationInstalled(s, k)])),
    ),
  )
}

export function useNavVisibility(): NavVisibilityContext {
  const installed = useInstalledIntegrations()
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  // 048: the Containers group follows the probe's answer. Before it lands the state is
  // `undefined` and the group is hidden, which is also what an absent runtime looks like — the
  // group never appears on a guess (FR-002).
  const runtime = useStore((s) => s.containerRuntime.presence?.state)
  return useMemo(
    () => ({ hosted, desktop: !hosted, installed: (key: SectionKey) => installed[key], runtime }),
    [hosted, installed, runtime],
  )
}
