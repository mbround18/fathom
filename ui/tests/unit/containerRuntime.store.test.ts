/**
 * 048 T032: the container-runtime slice and the presence gate in front of the sidebar.
 *
 * Two things are worth pinning here. Staleness: a runtime that goes away mid-scene must leave the
 * last inventory on screen with a reason beside it, never blank it (FR-003). And visibility: the
 * group is absent until a probe has answered, and absent again if the answer is "no runtime" —
 * the same rendering for two different facts, which is exactly why `probed` exists.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { runtimeVisible } from '@/resources/nav'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import {
  ALL_CONTAINERS,
  adoptionUpdated,
  clusterLinksUpdated,
  inventory,
  inventoryUnavailable,
  project,
  projectGroup,
  inventoryUpdated,
  presenceUpdated,
  usageSampled,
} from '../fixtures/containers'

const rt = () => useStore.getState().containerRuntime

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

describe('container runtime slice', () => {
  it('reads nothing before a probe answers', () => {
    expect(rt().probed).toBe(false)
    expect(rt().presence).toBeUndefined()
    expect(rt().inventory).toBeUndefined()
  })

  it('records presence and marks the probe answered, including when there is no runtime', () => {
    ingest(presenceUpdated('absent'))
    expect(rt().probed).toBe(true)
    expect(rt().presence?.state).toBe('absent')
    expect(rt().presence?.endpoint).toBeNull()
  })

  it('keeps the runtime’s own words for an unreachable socket', () => {
    ingest(presenceUpdated('unreachable'))
    expect(rt().presence?.detail).toContain('permission denied')
    expect(rt().presence?.endpoint).toBe('unix:///var/run/docker.sock')
  })

  it('marks the inventory stale without blanking it, and un-stales only on new facts', () => {
    const inv = inventory({
      projects: [project({ group: projectGroup('shop'), containers: ALL_CONTAINERS })],
    })
    ingest(inventoryUpdated(inv))
    ingest(
      inventoryUnavailable('Cannot connect to the Docker daemon at unix:///var/run/docker.sock'),
    )
    expect(rt().stale).toContain('Cannot connect')
    expect(rt().inventory?.projects[0]?.containers).toHaveLength(ALL_CONTAINERS.length)

    ingest(inventoryUpdated(inv))
    expect(rt().stale).toBeUndefined()
  })

  it('records adoption and the fallback that explains a refusal', () => {
    ingest(adoptionUpdated(['shop'], true))
    expect(rt().adopted).toEqual(['shop'])
    expect(rt().adoptionFellBack).toBe(true)
  })

  it('records the cluster links both directions share', () => {
    ingest(
      clusterLinksUpdated([
        { cluster: 'fathom-dev', container_ids: ['a'], managed_by_fathom: true },
      ]),
    )
    expect(rt().clusterLinks[0]?.cluster).toBe('fathom-dev')
  })

  it('leaves the events their own tasks render alone, rather than storing them half-way', () => {
    const before = rt()
    ingest(usageSampled([]))
    expect(rt()).toBe(before)
  })
})

describe('sidebar presence gate (FR-002)', () => {
  const ctx = (over: Partial<Parameters<typeof runtimeVisible>[0]> = {}) => ({
    hosted: false,
    desktop: true,
    installed: () => undefined,
    ...over,
  })

  it('hides the group until the probe has answered', () => {
    expect(runtimeVisible(ctx())).toBe(false)
  })

  it('hides the group when there is no runtime on this computer', () => {
    expect(runtimeVisible(ctx({ runtime: 'absent' }))).toBe(false)
  })

  it('shows the group for a runtime that is installed but stopped', () => {
    expect(runtimeVisible(ctx({ runtime: 'stopped' }))).toBe(true)
    expect(runtimeVisible(ctx({ runtime: 'unreachable' }))).toBe(true)
    expect(runtimeVisible(ctx({ runtime: 'ready' }))).toBe(true)
  })

  it('never shows the group in hosted mode: a server pod has no local daemon', () => {
    expect(runtimeVisible(ctx({ hosted: true, desktop: false, runtime: 'ready' }))).toBe(false)
  })
})
