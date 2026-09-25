/**
 * 020-gatekeeper-policy-pane: `gatekeeper` store slice + `GatekeeperPanel` rendering
 * (all four install states, awaiting-status vs zero, drill-down, truncation, Inspector link-out).
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { GatekeeperPanel } from '@/components/panels/GatekeeperPanel'
import type { GatekeeperConstraintSummary, GatekeeperOverview } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { ingest } from '@/store/ingest'

import { batch, system } from '../fixtures/builders'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const send = vi.fn()

function constraint(over: Partial<GatekeeperConstraintSummary> = {}): GatekeeperConstraintSummary {
  return {
    kind: 'K8sRequiredLabels',
    name: 'require-team',
    enforcement_action: 'deny',
    total_violations: 1,
    violations: [
      {
        section: 'workloads/pods',
        kind: 'Pod',
        namespace: 'default',
        name: 'nginx',
        enforcement_action: 'deny',
        message: 'missing label team',
      },
    ],
    violations_truncated: false,
    template_missing: false,
    ...over,
  }
}

function setOverview(o: GatekeeperOverview) {
  useStore.setState({ gatekeeper: { overview: o } })
}

beforeEach(() => {
  send.mockReset()
  useStore.setState(initialData())
  useStore.setState({ send })
})

describe('gatekeeper store slice', () => {
  it('starts empty, sends RefreshGatekeeper, and stores GatekeeperOverviewUpdated', () => {
    expect(useStore.getState().gatekeeper.overview).toBeUndefined()
    useStore.getState().refreshGatekeeper()
    expect(send).toHaveBeenCalledWith({ type: 'RefreshGatekeeper' })

    const overview: GatekeeperOverview = { state: 'Active', error: '', constraints: [constraint()] }
    ingest(batch([system(1, { type: 'GatekeeperOverviewUpdated', data: overview })]))
    expect(useStore.getState().gatekeeper.overview).toEqual(overview)
  })
})

describe('GatekeeperPanel', () => {
  it('shows waiting state and requests a refresh on mount', () => {
    render(<GatekeeperPanel />)
    expect(screen.getByText(/Waiting for the Gatekeeper scan/)).toBeTruthy()
    expect(send).toHaveBeenCalledWith({ type: 'RefreshGatekeeper' })
  })

  it('renders distinct messages for each non-active state', () => {
    setOverview({ state: 'NotInstalled', error: '', constraints: [] })
    const { unmount } = render(<GatekeeperPanel />)
    expect(screen.getByText('OPA Gatekeeper is not installed on this cluster')).toBeTruthy()
    unmount()

    setOverview({ state: 'InstalledNoConstraints', error: '', constraints: [] })
    const second = render(<GatekeeperPanel />)
    expect(screen.getByText(/installed but no constraints are defined/)).toBeTruthy()
    expect(screen.queryByText(/not installed on this cluster/)).toBeNull()
    second.unmount()

    setOverview({ state: 'PermissionDenied', error: 'forbidden: cannot list K8sFoo', constraints: [] })
    render(<GatekeeperPanel />)
    expect(screen.getByText(/forbidden: cannot list K8sFoo/)).toBeTruthy()
  })

  it('distinguishes awaiting status from a real count', () => {
    setOverview({
      state: 'Active',
      error: '',
      constraints: [
        constraint({ name: 'pending', total_violations: null, violations: [] }),
        constraint({ name: 'clean', total_violations: 0, violations: [] }),
        constraint({ name: 'bad', total_violations: 3 }),
      ],
    })
    render(<GatekeeperPanel />)
    expect(screen.getByText(/awaiting status/)).toBeTruthy()
    expect(screen.getByText(/0 violations/)).toBeTruthy()
    expect(screen.getByText(/3 violations/)).toBeTruthy()
  })

  it('drills into a constraint, flags truncation, and links out to the Inspector', () => {
    const openInspector = vi.fn()
    useStore.setState({ openInspector })
    setOverview({
      state: 'Active',
      error: '',
      constraints: [constraint({ total_violations: 143, violations_truncated: true })],
    })
    render(<GatekeeperPanel />)
    expect(screen.queryByTestId('gatekeeper-violations')).toBeNull()

    fireEvent.click(screen.getByTestId('gatekeeper-constraint'))
    expect(screen.getByText('showing 1 of 143 violations')).toBeTruthy()
    expect(screen.getByText(/missing label team/)).toBeTruthy()

    fireEvent.click(screen.getByTestId('gatekeeper-violation'))
    expect(openInspector).toHaveBeenCalledWith(
      { section: 'workloads/pods', kind: 'Pod', namespace: 'default', name: 'nginx' },
      undefined,
      true,
    )
  })

  it('omits the truncation notice when complete and flags a missing template', () => {
    setOverview({
      state: 'Active',
      error: '',
      constraints: [constraint({ template_missing: true })],
    })
    render(<GatekeeperPanel />)
    expect(screen.getByText(/references a missing ConstraintTemplate/)).toBeTruthy()
    fireEvent.click(screen.getByTestId('gatekeeper-constraint'))
    expect(screen.queryByText(/showing/)).toBeNull()
  })
})
