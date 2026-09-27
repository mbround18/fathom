/** 043 T002: every SetupState transition in data-model.md. */
import { describe, expect, it } from 'vitest'

import { setupStateOf, type SetupInputs } from '@/components/setup/setup-state'
import type { ConnectionProblem, KubeconfigIssue } from '@/protocol/types'

const PATHS = ['/home/op/.kube/config']
const CONTEXTS = ['kind-a', 'prod-eu']
const UNREACHABLE: ConnectionProblem = {
  kind: 'Unreachable',
  server: 'https://127.0.0.1:6443',
  detail: 'connection refused',
}

function inputs(over: Partial<SetupInputs> = {}): SetupInputs {
  return {
    kubeconfig: { loaded: true, contexts: CONTEXTS, paths: PATHS },
    hosted: false,
    ...over,
  }
}

const issue = (kind: KubeconfigIssue['kind'], paths = PATHS): KubeconfigIssue => ({
  kind,
  paths,
  detail: kind === 'Malformed' ? 'line 3: bad indent' : '',
})

describe('setupStateOf', () => {
  it('renders the normal shell before the kubeconfig status arrives', () => {
    expect(
      setupStateOf(inputs({ kubeconfig: { loaded: false, contexts: [], paths: [] } })),
    ).toBeUndefined()
  })

  it.each(['Missing', 'Empty', 'Malformed', 'Unreadable'] as const)(
    'shows the kubeconfig state for a %s kubeconfig',
    (kind) => {
      const s = setupStateOf(
        inputs({ kubeconfig: { loaded: true, contexts: [], paths: [], issue: issue(kind) } }),
      )
      expect(s).toEqual({ kind: 'kubeconfig', issue: issue(kind), paths: PATHS })
    },
  )

  it('treats a kubeconfig with no contexts and no issue as empty', () => {
    const s = setupStateOf(inputs({ kubeconfig: { loaded: true, contexts: [], paths: PATHS } }))
    expect(s).toMatchObject({ kind: 'kubeconfig', issue: { kind: 'Empty' }, paths: PATHS })
  })

  it('falls back to the files read when the issue names no path', () => {
    const s = setupStateOf(
      inputs({
        kubeconfig: { loaded: true, contexts: [], paths: PATHS, issue: issue('Empty', []) },
      }),
    )
    expect(s).toMatchObject({ kind: 'kubeconfig', paths: PATHS })
  })

  it('offers the pick when contexts exist but none is selected', () => {
    expect(setupStateOf(inputs())).toEqual({ kind: 'pick', contexts: CONTEXTS, paths: PATHS })
  })

  it('offers the pick when the current-context names a context that does not exist', () => {
    const s = setupStateOf(
      inputs({
        kubeconfig: {
          loaded: true,
          contexts: CONTEXTS,
          paths: PATHS,
          issue: { kind: 'NoCurrentContext', paths: PATHS, detail: 'current-context "gone"' },
        },
      }),
    )
    expect(s).toEqual({ kind: 'pick', contexts: CONTEXTS, paths: PATHS })
  })

  it('renders the shell once a context is selected and connected', () => {
    expect(
      setupStateOf(inputs({ activeContext: 'kind-a', connection: { connected: true } })),
    ).toBeUndefined()
  })

  it('leaves a connection problem with no setup pick to the per-pane state', () => {
    const s = setupStateOf(
      inputs({ activeContext: 'kind-a', connection: { connected: false, problem: UNREACHABLE } }),
    )
    expect(s).toBeUndefined()
  })

  describe('after a pick in setup', () => {
    it('pick → connecting while SelectContext is in flight', () => {
      expect(setupStateOf(inputs({ pickedInSetup: 'kind-a' }))).toEqual({
        kind: 'connecting',
        context: 'kind-a',
      })
    })

    it('connecting while the selected context has not answered yet', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: false },
        }),
      )
      expect(s).toEqual({ kind: 'connecting', context: 'kind-a' })
    })

    it('connecting → shell once connected', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: true },
        }),
      )
      expect(s).toBeUndefined()
    })

    it('connecting → failed on a connection problem', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: false, problem: UNREACHABLE },
        }),
      )
      expect(s).toEqual({ kind: 'failed', context: 'kind-a', problem: UNREACHABLE })
    })

    it('failed → connecting on Retry (the problem clears)', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: false },
        }),
      )
      expect(s).toEqual({ kind: 'connecting', context: 'kind-a' })
    })

    it('failed → sign-in when the operator chooses to sign in', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: false, problem: { ...UNREACHABLE, kind: 'Unauthorized' } },
          signInStep: true,
        }),
      )
      expect(s).toEqual({ kind: 'sign-in', context: 'kind-a' })
    })

    it('failed → pick when the operator chooses another context (the pick clears)', () => {
      expect(setupStateOf(inputs({ pickedInSetup: undefined }))).toMatchObject({ kind: 'pick' })
    })

    it('shows the sign-in step before the shell while it is offered', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: true },
          signInStep: true,
        }),
      )
      expect(s).toEqual({ kind: 'sign-in', context: 'kind-a' })
    })

    it('sign-in → shell on Skip', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: true },
          signInStep: false,
        }),
      )
      expect(s).toBeUndefined()
    })

    it('ignores a pick that was removed from the kubeconfig', () => {
      expect(setupStateOf(inputs({ pickedInSetup: 'deleted' }))).toMatchObject({ kind: 'pick' })
    })

    it('ignores a pick superseded by another selected context', () => {
      const s = setupStateOf(
        inputs({
          pickedInSetup: 'kind-a',
          activeContext: 'prod-eu',
          connection: { connected: false, problem: UNREACHABLE },
        }),
      )
      expect(s).toBeUndefined()
    })

    it('shows the kubeconfig state when a reload breaks the kubeconfig', () => {
      const s = setupStateOf(
        inputs({
          kubeconfig: { loaded: true, contexts: [], paths: PATHS, issue: issue('Malformed') },
          pickedInSetup: 'kind-a',
        }),
      )
      expect(s).toMatchObject({ kind: 'kubeconfig', issue: { kind: 'Malformed' } })
    })
  })

  describe('hosted mode', () => {
    it.each<[string, Partial<SetupInputs>]>([
      [
        'no kubeconfig',
        { kubeconfig: { loaded: true, contexts: [], paths: [], issue: issue('Missing') } },
      ],
      ['nothing selected', {}],
      ['a setup pick', { pickedInSetup: 'kind-a' }],
      [
        'a failed setup pick',
        {
          pickedInSetup: 'kind-a',
          activeContext: 'kind-a',
          connection: { connected: false, problem: UNREACHABLE },
        },
      ],
      ['the sign-in step', { pickedInSetup: 'kind-a', signInStep: true }],
    ])('never shows setup with %s', (_name, over) => {
      expect(setupStateOf(inputs({ ...over, hosted: true }))).toBeUndefined()
    })
  })
})
