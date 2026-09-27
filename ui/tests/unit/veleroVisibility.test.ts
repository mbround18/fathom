/** 043 T008: Velero is hidden unless detected as installed (FR-036); others always listed. */
import { describe, expect, it } from 'vitest'

import {
  VELERO_SECTION,
  entryVisible,
  integrationVisible,
  navEntry,
  type NavVisibilityContext,
} from '@/resources/nav'
import { buildSectionCandidates, buildToolCandidates } from '@/resources/paletteIndex'
import { ALL_TOOL_TABS, visibleToolTabs } from '@/components/shell/stack-tabs'

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

describe('Velero on every listing surface (043 T071)', () => {
  const sections = (v: boolean | undefined) =>
    buildSectionCandidates(ctx(v)).map((r) => (r.payload as { section: string }).section)
  const tools = (v: boolean | undefined) =>
    buildToolCandidates(ctx(v)).map((r) => (r.payload as { tool: string }).tool)
  const tabs = (v: boolean | undefined) => visibleToolTabs(ALL_TOOL_TABS, v).map((t) => t.tool)

  it.each([undefined, false])('hides Velero from palette and tool tabs when %s', (v) => {
    expect(sections(v)).not.toContain(VELERO_SECTION)
    expect(tools(v)).not.toContain('velero')
    expect(tabs(v)).not.toContain('velero')
    // Every other integration keeps its entry.
    expect(sections(v)).toContain('integrations/knative')
    expect(tools(v)).toContain('knative')
    expect(tabs(v)).toContain('certManager')
  })

  it('lists Velero everywhere once detected', () => {
    expect(sections(true)).toContain(VELERO_SECTION)
    expect(tools(true)).toContain('velero')
    expect(tabs(true)).toContain('velero')
  })
})

describe('palette descriptions (043 FR-021)', () => {
  it('carries each navigator entry description, including "More" kinds', () => {
    const results = buildSectionCandidates()
    for (const r of results) expect(r.description, r.id).toBeTruthy()
    const rs = results.find((r) => r.label === 'ReplicaSets')
    expect(rs?.description).toBe(navEntry('workloads/replicasets')?.description)
  })
})
