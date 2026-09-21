/**
 * T059: every registered resource descriptor honours the column-budget and wiring contract
 * (data-model.md §5, contracts/layout.md § Column budgets).
 */
import { describe, expect, it } from 'vitest'

import type { Route } from '@/protocol/types'
import { ALL_DESCRIPTORS, descriptorFor } from '@/resources'

import { KIND_LIST_EVENTS } from '../fixtures/kinds'

/** A 390 px phone minus the 16 px gutters on each side. */
const PHONE_BUDGET = 358

describe.each(ALL_DESCRIPTORS.map((d) => [d.section, d] as const))('%s', (section, d) => {
  const route: Route = { section, scope: 'Namespaced', context: 'kind-a', namespace: null }
  const event = KIND_LIST_EVENTS[section]

  it('is registered under its own section', () => {
    expect(descriptorFor(section)).toBe(d)
  })

  it('has priority-1 name and status columns', () => {
    const p1 = d.columns.filter((c) => c.priority === 1).map((c) => c.id)
    expect(p1).toContain('name')
    expect(p1).toContain('status')
  })

  it('fits its priority-1 columns in a phone pane', () => {
    const sum = d.columns.filter((c) => c.priority === 1).reduce((n, c) => n + c.minWidth, 0)
    expect(sum).toBeLessThanOrEqual(PHONE_BUDGET)
  })

  it('has unique column ids and positive minWidths', () => {
    const ids = d.columns.map((c) => c.id)
    expect(new Set(ids).size).toBe(ids.length)
    for (const c of d.columns) expect(c.minWidth).toBeGreaterThan(0)
  })

  it('has a list fixture', () => {
    expect(event, `add a fixture for ${section} to tests/fixtures/kinds.ts`).toBeDefined()
  })

  it('selects its rows from its list event and ignores others', () => {
    if (!event) return
    const rows = d.selectListEvent(event)
    expect(rows).toBeDefined()
    expect(rows?.length).toBeGreaterThan(0)
    expect(d.selectListEvent({ type: 'NamespacesUpdated', data: ['x'] })).toBeUndefined()
  })

  it('has a rowKey that is unique on its fixture', () => {
    if (!event) return
    const rows = d.selectListEvent(event) ?? []
    const keys = rows.map((r) => d.rowKey(r))
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('refresh(route) returns a Refresh command', () => {
    const cmd = d.refresh(route)
    expect(cmd.type).toMatch(/^Refresh[A-Z]/)
  })

  it('read(ref) asks for the object and its events', () => {
    if (!event || !d.read) return
    const first = (d.selectListEvent(event) ?? [])[0]
    expect(first).toBeDefined()
    if (!first) return
    const ref = d.ref(first)
    expect(ref.section).toBe(section)
    expect(ref.kind).toBe(d.kind)
    const cmds = d.read(ref).map((c) => c.type)
    expect(cmds.some((t) => t.startsWith('Select'))).toBe(true)
    expect(cmds).toContain('RefreshEvents')
  })

  it('renders every cell without throwing', () => {
    if (!event) return
    for (const r of (d.selectListEvent(event) ?? []).slice(0, 20)) {
      for (const c of d.columns) {
        expect(() => c.cell(r)).not.toThrow()
        expect(typeof c.text(r)).toBe('string')
      }
    }
  })

  it('has detail sections for metadata, status, spec, and labels/annotations', () => {
    const ids = d.detailSections.map((s) => s.id)
    for (const want of ['metadata', 'status', 'spec', 'labels', 'annotations']) {
      expect(ids).toContain(want)
    }
  })
})

describe('registry', () => {
  it('includes pods and deployments (the MVP kinds)', () => {
    expect(descriptorFor('workloads/pods')?.kind).toBe('Pod')
    expect(descriptorFor('workloads/deployments')?.kind).toBe('Deployment')
  })

  it('returns undefined for an unknown section', () => {
    expect(descriptorFor('nope/nothing')).toBeUndefined()
  })
})
