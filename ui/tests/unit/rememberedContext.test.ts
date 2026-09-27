/**
 * 043 T013: a context picked on the setup screen is remembered in Fathom's UI config (never the
 * kubeconfig) and reopened at startup, but only when the kubeconfig selects none and the
 * remembered context still exists.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import type { BackendCmd, EventBatch, UiConfig } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import { contexts, NO_KUBECONFIG_PATH, SAVED_UI } from '../fixtures'
import { batch, system } from '../fixtures/builders'

let sent: BackendCmd[] = []

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  sent = []
  useStore.setState({ send: (cmd: BackendCmd) => void sent.push(cmd) })
})

const remembering = (context: string): EventBatch =>
  batch([
    system(2, {
      type: 'UiConfigLoaded',
      data: {
        ui: { ...SAVED_UI, lastSelection: { context, section: 'overview' } } satisfies UiConfig,
        expanded_groups: ['Workloads'],
        mcp_stdio_enabled: false,
      },
    }),
  ])

const noCurrent = (seq = 4): EventBatch =>
  batch([
    system(seq, {
      type: 'KubeconfigStatus',
      data: {
        issue: { kind: 'NoCurrentContext', paths: [NO_KUBECONFIG_PATH], detail: '' },
        paths: [NO_KUBECONFIG_PATH],
      },
    }),
  ])

const setContexts = () => sent.filter((c) => c.type === 'SetContext')

describe('rememberContext', () => {
  it('saves the pick as lastSelection in the UI config', () => {
    ingest(remembering('kind-a'))
    useStore.getState().rememberContext('kind-b')
    const save = sent.find((c) => c.type === 'SaveUiConfig')
    expect(save).toMatchObject({
      type: 'SaveUiConfig',
      data: {
        ui: { lastSelection: { context: 'kind-b', section: 'overview' }, theme: SAVED_UI.theme },
        expanded_groups: ['Workloads'],
      },
    })
    expect(useStore.getState().ui.lastSelection?.context).toBe('kind-b')
  })

  it('pickInSetup selects, remembers and records the pick', () => {
    ingest(contexts(null))
    useStore.getState().pickInSetup('kind-b')
    expect(setContexts()).toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
    expect(sent.some((c) => c.type === 'SaveUiConfig')).toBe(true)
    expect(useStore.getState().setup.picked).toBe('kind-b')
    // No identity provider: no sign-in step.
    expect(useStore.getState().setup.signInStep).toBe(false)
  })

  it('re-picking the context that failed retries it', () => {
    ingest(contexts('kind-a'))
    useStore.getState().pickInSetup('kind-a')
    expect(setContexts()).toEqual([{ type: 'SetContext', data: { context: 'kind-a' } }])
  })
})

describe('startup context', () => {
  it('opens the remembered context when the kubeconfig selects none', () => {
    ingest(remembering('kind-b'))
    ingest(contexts(null))
    ingest(noCurrent())
    expect(setContexts()).toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
  })

  it('works whichever of the UI config and the contexts arrives first', () => {
    ingest(contexts(null))
    ingest(remembering('kind-b'))
    expect(setContexts()).toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
  })

  it("the kubeconfig's current-context wins", () => {
    ingest(remembering('kind-b'))
    ingest(contexts('kind-a'))
    expect(setContexts()).toEqual([])
    expect(useStore.getState().session.activeContext).toBe('kind-a')
  })

  it('a remembered context that was removed from the kubeconfig selects nothing', () => {
    ingest(remembering('gone-prod'))
    ingest(contexts(null))
    ingest(noCurrent())
    expect(setContexts()).toEqual([])
    expect(useStore.getState().session.activeContext).toBeUndefined()
  })

  it('never applies in hosted mode', () => {
    useStore.setState({ hosted: { ...useStore.getState().hosted, mode: 'hosted' } })
    ingest(remembering('kind-b'))
    ingest(contexts(null))
    expect(setContexts()).toEqual([])
  })

  it('is tried once: backing out to the setup screen later does not reopen it', () => {
    ingest(remembering('kind-b'))
    ingest(contexts(null))
    ingest(contexts(null, 6))
    expect(setContexts()).toHaveLength(1)
  })
})
