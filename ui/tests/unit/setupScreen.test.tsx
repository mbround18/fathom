/** 043 T011: the setup screen's context list (filter, keyboard, virtualisation). */
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { SetupScreen, VIRTUALIZE_ABOVE } from '@/components/setup/SetupScreen'
import type { BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'

let sent: BackendCmd[] = []

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  sent = []
  useStore.setState({ send: (cmd: BackendCmd) => void sent.push(cmd) })
})

const pick = (contexts: string[]) =>
  render(<SetupScreen state={{ kind: 'pick', contexts, paths: ['/home/op/.kube/config'] }} />)

describe('SetupScreen pick', () => {
  it('lists every context verbatim, filters, and picks with the keyboard', () => {
    useStore.setState((s) => ({
      session: { ...s.session, contexts: ['kind-a', 'arn:aws:eks:eu/prod'], contextsLoaded: true },
    }))
    pick(['kind-a', 'arn:aws:eks:eu/prod'])
    const box = screen.getByRole('searchbox', { name: 'Filter contexts' })
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual([
      'kind-a',
      'arn:aws:eks:eu/prod',
    ])
    fireEvent.change(box, { target: { value: 'EKS' } })
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual(['arn:aws:eks:eu/prod'])
    fireEvent.keyDown(box, { key: 'Enter' })
    expect(sent.filter((c) => c.type === 'SetContext')).toEqual([
      { type: 'SetContext', data: { context: 'arn:aws:eks:eu/prod' } },
    ])
    expect(sent.some((c) => c.type === 'SaveUiConfig')).toBe(true)
  })

  it(`renders only the rows in view above ${String(VIRTUALIZE_ABOVE)} contexts`, () => {
    const many = Array.from({ length: VIRTUALIZE_ABOVE + 50 }, (_, i) => `ctx-${String(i)}`)
    pick(many)
    expect(screen.getAllByText(/contexts in your kubeconfig/)[0]?.textContent).toContain(
      String(many.length),
    )
    expect(screen.queryAllByRole('option').length).toBeLessThan(many.length)
  })
})
