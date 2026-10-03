/**
 * 048-docker-management US4: the read that makes the cluster→container direction possible.
 *
 * Its own module because the cluster view imports it alongside a component, and a file that
 * exports both loses fast refresh.
 */
import { useEffect } from 'react'

import { useStore } from '@/store'

/**
 * Asks the daemon once for the containers behind the clusters on screen.
 *
 * The containers scene reads the inventory when it opens (Principle V: nothing polls a scene
 * nobody is looking at), so a cluster view opened first would have no link to read. One read on
 * mount, and only when a runtime answered the probe — on a machine with no Docker there is
 * nothing to ask.
 */
export function useBackingContainers(): void {
  const presence = useStore((s) => s.containerRuntime.presence)
  const refresh = useStore((s) => s.refreshContainerInventory)
  const ready = presence?.state === 'ready'
  useEffect(() => {
    if (ready) refresh(['container'])
  }, [ready, refresh])
}
