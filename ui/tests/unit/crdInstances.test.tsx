/**
 * #36: a Custom Resource kind's instance list always ends in a clear state — the list, "No
 * instances", "Permission denied", not found, or an error with Retry — never an endless spinner,
 * and nothing from the previous cluster survives a context switch.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import type { BackendCmd, CrdCatalogEntry, CrdListFailure } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions, setActionTimeout } from '@/store/actions'
import { ingest } from '@/store/ingest'
import { CustomResourcesSection } from '@/components/nav/CustomResourcesSection'

import { twoContexts } from '../fixtures'
import { batch, clusterRoute, envelope, globalRoute } from '../fixtures/builders'

const ORDER: CrdCatalogEntry = {
  group: 'acme.cert-manager.io',
  kind: 'Order',
  plural: 'orders',
  versions: ['v1'],
  scope: 'Namespaced',
  permission_denied: false,
}
const KEY = 'acme.cert-manager.io/Order'
const SECTION = 'custom-resources'

let sent: BackendCmd[]
let nextId: number

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  ingest(twoContexts)
  sent = []
  nextId = 500
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(nextId++)
    },
    send: (cmd) => {
      sent.push(cmd)
    },
  })
})

function catalog(context = 'kind-a') {
  ingest(
    batch([
      envelope(10, clusterRoute(context, SECTION), {
        type: 'System',
        data: { type: 'CrdCatalogUpdated', data: { entries: [ORDER] } },
      }),
    ]),
  )
}

function instances(
  items: string[],
  failure: CrdListFailure | null = null,
  context = 'kind-a',
  seq = 11,
) {
  ingest(
    batch([
      envelope(seq, clusterRoute(context, SECTION), {
        type: 'System',
        data: {
          type: 'CrdInstancesUpdated',
          data: {
            group: ORDER.group,
            kind: ORDER.kind,
            version: 'v1',
            namespace: null,
            instances: items,
            failure,
          },
        },
      }),
    ]),
  )
}

function finished(id: number, ok: boolean, error: string | null = null) {
  ingest(
    batch([
      envelope(
        40,
        globalRoute(),
        { type: 'CommandFinished', data: { ok, error } },
        { type: 'Request', data: id },
      ),
    ]),
  )
}

function openOrder() {
  catalog()
  render(<CustomResourcesSection />)
  fireEvent.click(screen.getByRole('option', { name: /Order/ }))
}

describe('Custom Resources instance list (#36)', () => {
  it('selecting a kind asks for its instances once and shows them', async () => {
    openOrder()
    expect(sent.filter((c) => c.type === 'RefreshCrdInstances')).toEqual([
      {
        type: 'RefreshCrdInstances',
        data: { group: ORDER.group, kind: 'Order', version: 'v1', namespace: null },
      },
    ])
    expect(screen.getByRole('status', { name: 'Loading Order instances' })).toBeTruthy()
    instances([JSON.stringify({ metadata: { name: 'web-tls-1', namespace: 'demo' } })])
    await screen.findByText('web-tls-1')
  })

  it('an empty list says there are none', async () => {
    openOrder()
    instances([])
    await screen.findByText('No Order instances found.')
  })

  it('a 403 says permission denied with the server message', async () => {
    openOrder()
    instances([], { type: 'Forbidden', data: { message: 'orders is forbidden' } })
    await screen.findByText('Permission denied')
    expect(screen.getByText(/orders is forbidden/)).toBeTruthy()
    expect(useStore.getState().crds.instances[KEY]).toMatchObject({ status: 'forbidden' })
  })

  it('a kind the cluster does not serve says not found', async () => {
    openOrder()
    instances([], { type: 'NotFound', data: { message: 'the server could not find it' } })
    await screen.findByText('Order not found')
  })

  it('any other failure shows the error and Retry asks again', async () => {
    openOrder()
    instances([], { type: 'Failed', data: { message: 'conversion webhook failed' } })
    await screen.findByText('Could not list Order instances')
    expect(screen.getByText('conversion webhook failed')).toBeTruthy()
    sent = []
    fireEvent.click(screen.getByRole('button', { name: /Retry/ }))
    expect(sent.map((c) => c.type)).toEqual(['RefreshCrdInstances'])
    expect(screen.getByRole('status', { name: 'Loading Order instances' })).toBeTruthy()
  })

  it('a failed command without an answer ends the spinner with its error', async () => {
    openOrder()
    await waitFor(() => {
      expect(sent.some((c) => c.type === 'RefreshCrdInstances')).toBe(true)
    })
    finished(500, false, 'No cluster connection')
    await screen.findByText('No cluster connection')
  })

  it('an unanswered command ends the spinner instead of waiting forever', async () => {
    setActionTimeout(20)
    openOrder()
    await screen.findByText('Could not list Order instances')
    expect(screen.getByText(/did not answer/)).toBeTruthy()
  })
})

describe('Custom Resources across contexts (#36)', () => {
  it('drops an instance list for a context the operator left', () => {
    catalog()
    instances(['{}'], null, 'kind-b')
    expect(useStore.getState().crds.instances[KEY]).toBeUndefined()
  })

  it('switching context forgets the catalog and instance lists', () => {
    catalog()
    useStore.getState().selectCrdKind(ORDER)
    instances(['{}'])
    useStore.getState().selectContext('kind-b')
    const crds = useStore.getState().crds
    expect(crds.catalogLoaded).toBe(false)
    expect(crds.catalog).toEqual([])
    expect(crds.instances).toEqual({})
    expect(crds.selected).toBeUndefined()
  })

  it('an open section asks for the new cluster catalog after a switch', async () => {
    catalog()
    render(<CustomResourcesSection />)
    sent = []
    useStore.getState().selectContext('kind-b')
    await waitFor(() => {
      expect(sent.map((c) => c.type)).toContain('RefreshCrds')
    })
    expect(
      screen.getByRole('status', { name: 'Discovering Custom Resource Definitions' }),
    ).toBeTruthy()
  })
})
