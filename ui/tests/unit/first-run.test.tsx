/**
 * T126 first-run UX: the `KubeconfigStatus` / `ActiveConnectionProblem` handlers, the pane-state
 * selector, the copy, the kind-create auto-connect, and the `FirstRunState` component.
 */
import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FirstRunState, RELOAD_TIMEOUT_MS } from '@/components/shell/FirstRunState'
import {
  connectionProblemCopy,
  firstRunOf,
  kubeconfigIssueCopy,
  reloadOutcome,
  type FirstRun,
} from '@/lib/first-run'
import type { BackendCmd, KubeconfigIssue } from '@/protocol/types'
import { initialData, listKey, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import {
  connectionProblem,
  contextsWithProblem,
  kubeconfigMalformed,
  MALFORMED_DETAIL,
  NO_KUBECONFIG_PATH,
  noCurrentContext,
  noKubeconfig,
  twoContexts,
  UNREACHABLE_DETAIL,
} from '../fixtures'
import { batch, globalRoute, stream, system } from '../fixtures/builders'

let sent: BackendCmd[] = []

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  sent = []
  useStore.setState({ send: (cmd: BackendCmd) => void sent.push(cmd) })
})

const session = () => useStore.getState().session

describe('first-run handlers', () => {
  it('KubeconfigStatus records the issue and a null issue clears it', () => {
    ingest(noKubeconfig)
    expect(session().kubeconfigIssue?.kind).toBe('Missing')
    ingest(batch([system(9, { type: 'KubeconfigStatus', data: { issue: null, paths: [] } })]))
    expect(session().kubeconfigIssue).toBeUndefined()
  })

  it('ActiveConnectionProblem records per context, fails the loading lists, and clears on null', () => {
    ingest(contextsWithProblem('Unreachable'))
    expect(session().connectionProblems['kind-a']?.kind).toBe('Unreachable')
    // ContextChanged marked the lists loading after the problem: a later problem fails them so
    // nothing waits forever.
    ingest(connectionProblem('kind-a', 'Unreachable', 20))
    const pods = useStore.getState().lists[listKey('kind-a', 'workloads/pods', 'Namespaced', null)]
    expect(pods?.status).toBe('error')
    expect(pods?.error).toBe('Cannot reach kind-a')
    // The status bar no longer says "Connecting to cluster..." once the connect failed.
    expect(useStore.getState().conn.lastNotice).toMatchObject({
      level: 'warn',
      message: 'Cannot reach kind-a',
    })

    ingest(connectionProblem('kind-a', null, 21))
    expect(session().connectionProblems).toEqual({})
  })

  it('retryConnection clears the problem, marks lists loading and re-sends SetContext', () => {
    ingest(contextsWithProblem('Unauthorized'))
    useStore.getState().retryConnection()
    expect(session().connectionProblems).toEqual({})
    expect(sent).toEqual([{ type: 'SetContext', data: { context: 'kind-a' } }])
  })

  it('retryConnection without an active context re-reads the kubeconfig', () => {
    ingest(noKubeconfig)
    useStore.getState().retryConnection()
    expect(sent).toEqual([{ type: 'Initialize' }])
  })

  it('a kind cluster created at first run becomes the active context', () => {
    ingest(noKubeconfig)
    useStore.setState((s) => ({
      kind: {
        ...s.kind,
        operations: {
          r1: { requestId: 'r1', clusterName: 'dev', kind: 'create', status: 'in-progress' },
        },
      },
    }))
    ingest(
      batch([
        system(10, {
          type: 'ContextsUpdated',
          data: { contexts: ['kind-dev'], current: 'kind-dev' },
        }),
        system(11, { type: 'KubeconfigStatus', data: { issue: null, paths: [] } }),
        stream(12, globalRoute(), {
          type: 'KindClusterActionFinished',
          data: { request_id: 'r1', ok: true, error: null },
        }),
      ]),
    )
    expect(sent).toEqual([{ type: 'SetContext', data: { context: 'kind-dev' } }])
  })

  it('a kind cluster created while connected elsewhere does not switch context', () => {
    ingest(twoContexts)
    useStore.setState((s) => ({
      kind: {
        ...s.kind,
        operations: {
          r1: { requestId: 'r1', clusterName: 'dev', kind: 'create', status: 'in-progress' },
        },
      },
    }))
    ingest(
      batch([
        stream(12, globalRoute(), {
          type: 'KindClusterActionFinished',
          data: { request_id: 'r1', ok: true, error: null },
        }),
      ]),
    )
    expect(sent).toEqual([])
  })
})

describe('firstRunOf', () => {
  const base = { contextsLoaded: true, contextCount: 2, activeContext: 'a' }
  it('renders normally before contexts load and when healthy', () => {
    expect(firstRunOf({ ...base, contextsLoaded: false, contextCount: 0 })).toBeUndefined()
    expect(firstRunOf(base)).toBeUndefined()
  })
  it('a kubeconfig issue wins, and no contexts is an Empty issue', () => {
    const issue = { kind: 'Malformed' as const, paths: ['/k'], detail: 'x' }
    expect(firstRunOf({ ...base, issue })).toEqual({ kind: 'kubeconfig', issue })
    expect(firstRunOf({ ...base, contextCount: 0, activeContext: undefined })?.kind).toBe(
      'kubeconfig',
    )
  })
  it('no active context asks to pick one; a problem explains the connection', () => {
    expect(firstRunOf({ ...base, activeContext: undefined })).toEqual({ kind: 'pick' })
    const problem = { kind: 'Tls' as const, server: null, detail: 'd' }
    expect(firstRunOf({ ...base, problem })).toEqual({ kind: 'connection', context: 'a', problem })
  })
})

describe('copy', () => {
  it('names the searched paths and the parser detail', () => {
    expect(
      kubeconfigIssueCopy({ kind: 'Missing', paths: ['/a', '/b'], detail: '' }).detail,
    ).toContain('/a, /b')
    expect(
      kubeconfigIssueCopy({ kind: 'Malformed', paths: ['/a'], detail: MALFORMED_DETAIL }).detail,
    ).toContain(MALFORMED_DETAIL)
  })
  it('reload outcome names the paths for every result', () => {
    const missing: KubeconfigIssue = { kind: 'Missing', paths: ['/a', '/b'], detail: '' }
    expect(reloadOutcome({ issue: missing, paths: [], contextCount: 0 })).toBe(
      'No kubeconfig found; looked in /a, /b.',
    )
    expect(reloadOutcome({ paths: ['/k'], contextCount: 1 })).toBe('Found 1 context in /k.')
    expect(reloadOutcome({ paths: ['/k', '/j'], contextCount: 3 })).toBe(
      'Found 3 contexts in /k, /j.',
    )
    expect(
      reloadOutcome({
        issue: { kind: 'Malformed', paths: ['/k'], detail: 'x' },
        paths: ['/k'],
        contextCount: 0,
      }),
    ).toBe('Reloaded: /k could not be parsed.')
  })
  it('never headlines the raw error', () => {
    for (const kind of [
      'Unreachable',
      'Dns',
      'Timeout',
      'Tls',
      'Unauthorized',
      'Forbidden',
      'Other',
    ] as const) {
      const c = connectionProblemCopy('ctx', { kind, server: 'https://x', detail: 'RAW' })
      expect(c.title).not.toContain('RAW')
      expect(c.detail).not.toContain('RAW')
    }
  })
})

describe('FirstRunState', () => {
  it('no kubeconfig: reload and create-a-cluster actions', () => {
    ingest(noKubeconfig)
    const navigate = vi.fn()
    useStore.setState({ navigate })
    const state = firstRunOf({
      contextsLoaded: true,
      contextCount: 0,
      issue: session().kubeconfigIssue,
    }) as FirstRun
    render(<FirstRunState state={state} />)
    expect(screen.getByText('No kubeconfig found')).toBeTruthy()
    fireEvent.click(screen.getByTestId('first-run-reload'))
    expect(sent).toEqual([{ type: 'Initialize' }])
    fireEvent.click(screen.getByTestId('first-run-create-kind'))
    expect(navigate).toHaveBeenCalledWith('integrations/cluster-settings')
  })

  it('reload: pending until the next KubeconfigStatus, then the outcome even when unchanged', () => {
    ingest(noKubeconfig)
    const state = firstRunOf({
      contextsLoaded: true,
      contextCount: 0,
      issue: session().kubeconfigIssue,
    }) as FirstRun
    render(<FirstRunState state={state} />)
    expect(screen.getByTestId('first-run-paths').textContent).toContain(NO_KUBECONFIG_PATH)
    const button = screen.getByTestId('first-run-reload')
    fireEvent.click(button)
    expect(sent).toEqual([{ type: 'Initialize' }])
    expect(button.hasAttribute('disabled')).toBe(true)
    expect(button.textContent).toContain('Reloading')
    expect(screen.queryByTestId('first-run-reload-outcome')).toBeNull()

    // The backend answers with the same result: the button still reports it.
    act(() => ingest(noKubeconfig))
    expect(button.hasAttribute('disabled')).toBe(false)
    expect(button.textContent).toContain('Reload kubeconfig')
    expect(screen.getByTestId('first-run-reload-outcome').textContent).toBe(
      `No kubeconfig found; looked in ${NO_KUBECONFIG_PATH}.`,
    )
  })

  it('reload: gives up waiting after the timeout and says so', () => {
    vi.useFakeTimers()
    try {
      ingest(noKubeconfig)
      const issue = session().kubeconfigIssue
      if (!issue) throw new Error('no issue recorded')
      render(<FirstRunState state={{ kind: 'kubeconfig', issue }} />)
      const button = screen.getByTestId('first-run-reload')
      fireEvent.click(button)
      expect(button.hasAttribute('disabled')).toBe(true)
      act(() => {
        vi.advanceTimersByTime(RELOAD_TIMEOUT_MS)
      })
      expect(button.hasAttribute('disabled')).toBe(false)
      expect(screen.getByTestId('first-run-reload-outcome').textContent).toContain('No answer')
    } finally {
      vi.useRealTimers()
    }
  })

  it('pick: shows the kubeconfig the contexts came from and reports the count on reload', () => {
    ingest(noCurrentContext)
    render(<FirstRunState state={{ kind: 'pick' }} />)
    expect(screen.getByTestId('first-run-paths').textContent).toBe(
      `Kubeconfig: ${NO_KUBECONFIG_PATH}`,
    )
    fireEvent.click(screen.getByTestId('first-run-reload'))
    act(() => ingest(noCurrentContext))
    expect(screen.getByTestId('first-run-reload-outcome').textContent).toBe(
      `Found 2 contexts in ${NO_KUBECONFIG_PATH}.`,
    )
  })

  it('in Cluster Settings the create link is omitted (the kind panel is right there)', () => {
    ingest(kubeconfigMalformed)
    const issue = session().kubeconfigIssue
    if (!issue) throw new Error('no issue recorded')
    render(<FirstRunState state={{ kind: 'kubeconfig', issue }} inSettings />)
    expect(screen.queryByTestId('first-run-create-kind')).toBeNull()
  })

  it('pick: one button per context sends SetContext', () => {
    ingest(noCurrentContext)
    render(<FirstRunState state={{ kind: 'pick' }} />)
    const buttons = screen.getAllByTestId('first-run-context')
    expect(buttons.map((b) => b.textContent)).toEqual(['kind-a', 'kind-b'])
    const kindB = buttons[1]
    if (!kindB) throw new Error('no kind-b button')
    fireEvent.click(kindB)
    expect(sent).toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
  })

  it('connection: retry, the other contexts, and the verbatim detail', () => {
    ingest(contextsWithProblem('Unreachable'))
    const problem = session().connectionProblems['kind-a']
    if (!problem) throw new Error('no problem recorded')
    render(<FirstRunState state={{ kind: 'connection', context: 'kind-a', problem }} />)
    expect(screen.getByText('Cannot reach kind-a')).toBeTruthy()
    expect(screen.getByTestId('first-run-detail').textContent).toBe(UNREACHABLE_DETAIL)
    expect(screen.getAllByTestId('first-run-context').map((b) => b.textContent)).toEqual(['kind-b'])
    fireEvent.click(screen.getByTestId('first-run-retry'))
    expect(sent).toEqual([{ type: 'SetContext', data: { context: 'kind-a' } }])
  })
})
