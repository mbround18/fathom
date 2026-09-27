/** 043 T008: Velero is hidden unless detected as installed (FR-036); others always listed. */
import { describe, expect, it } from 'vitest'

import {
  VELERO_SECTION,
  entryVisible,
  integrationVisible,
  navEntry,
  type NavVisibilityContext,
} from '@/resources/nav'

const ctx = (velero: boolean | undefined): NavVisibilityContext => ({
  hosted: false,
  desktop: true,
  installed: (key) => (key === VELERO_SECTION ? velero : undefined),
})

describe('Velero visibility', () => {
  it.each<[string, boolean | undefined, boolean]>([
    ['unknown (not yet detected)', undefined, false],
    ['not installed', false, false],
    ['installed', true, true],
  ])('is %s → visible %s', (_name, installed, visible) => {
    expect(integrationVisible(VELERO_SECTION, installed)).toBe(visible)
    const e = navEntry(VELERO_SECTION)
    expect(e).toBeDefined()
    if (e) expect(entryVisible(e, ctx(installed))).toBe(visible)
  })

  it.each([undefined, false, true])('never hides another integration (installed = %s)', (i) => {
    for (const key of ['integrations/knative', 'integrations/keda', 'integrations/cert-manager']) {
      expect(integrationVisible(key, i), key).toBe(true)
      const e = navEntry(key)
      if (e) expect(entryVisible(e, ctx(i)), key).toBe(true)
    }
  })
})
