/**
 * 031: a namespace-scoped identity. A section the API server denies (403) is recorded in
 * `denied` and its loading lists fail as `forbidden` (the pane shows "Not permitted"), never as
 * a toast; a later successful list clears it.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { deniedKey, initialData, listKey, useStore } from '@/store'
import { ingest } from '@/store/ingest'

import { twoContexts } from '../fixtures'
import { batch, envelope, list, nsRoute, system } from '../fixtures/builders'

const SECTION = 'config/configmaps'
const KEY = listKey('kind-a', SECTION, 'Namespaced', null)
const DENIAL = 'configmaps is forbidden: User "system:serviceaccount:x:y" cannot list resource'

function denied(seq: number, correlation?: number) {
  return envelope(
    seq,
    nsRoute('kind-a', SECTION),
    { type: 'System', data: { type: 'AccessDenied', data: { message: DENIAL } } },
    correlation === undefined ? undefined : { type: 'Request', data: correlation },
  )
}

beforeEach(() => {
  useStore.setState(initialData())
})

describe('AccessDenied', () => {
  it('marks the section denied and fails its loading list as forbidden, without a notice', () => {
    ingest(twoContexts)
    useStore.getState().refreshSection(SECTION)
    ingest(batch([denied(10, 7)]))
    const s = useStore.getState()
    expect(s.denied[deniedKey('kind-a', SECTION)]).toBe(DENIAL)
    expect(s.lists[KEY]).toMatchObject({ status: 'error', forbidden: true, error: DENIAL })
    expect(s.conn.lastNotice).toBeUndefined()
  })

  it('keys a context-less command route by the active context', () => {
    ingest(twoContexts)
    const base = denied(10)
    ingest(batch([{ ...base, route: { ...base.route, context: null } }]))
    expect(useStore.getState().denied[deniedKey('kind-a', SECTION)]).toBe(DENIAL)
  })

  it("ignores a pending action's own denial (its toast reports it)", () => {
    ingest(twoContexts)
    useStore.setState({ pending: { 7: { label: 'Delete', paneId: 'list-1', sentAt: 0 } } })
    ingest(batch([denied(10, 7)]))
    expect(useStore.getState().denied).toEqual({})
  })

  it('a later successful list clears the denial', () => {
    ingest(twoContexts)
    ingest(batch([denied(10)]))
    ingest(batch([list(11, nsRoute('kind-a', SECTION), { type: 'ConfigMapsUpdated', data: [] })]))
    const s = useStore.getState()
    expect(s.denied).toEqual({})
    expect(s.lists[KEY]?.status).toBe('ready')
  })

  it('changing namespace clears every denial', () => {
    ingest(twoContexts)
    ingest(batch([denied(10)]))
    useStore.getState().selectNamespace('demo')
    expect(useStore.getState().denied).toEqual({})
  })
})

describe('NamespaceAccess', () => {
  it('records whether the namespace list is restricted', () => {
    ingest(twoContexts)
    ingest(batch([system(10, { type: 'NamespaceAccess', data: { restricted: true } })]))
    expect(useStore.getState().session.namespacesRestricted).toBe(true)
  })
})
