/** 018: store merge of access results + `useMutationAccess` unknown/denied/allowed semantics. */
import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { RbacAccessResult } from '@/protocol/types'
import { useMutationAccess, deniedTitle } from '@/hooks/useMutationAccess'
import { initialData, useStore } from '@/store'
import { mergeAccessResults } from '@/store/handlers/rbacAccess'

const res = (resource: string, allowed: boolean, reason: string | null = null): RbacAccessResult => ({
  check: { verb: 'update', resource, namespace: null, group: '' },
  allowed,
  reason,
})

describe('mergeAccessResults', () => {
  it('keeps unrelated results and replaces same-key results', () => {
    const merged = mergeAccessResults([res('nodes', true), res('secrets', true)], [res('nodes', false)])
    expect(merged).toHaveLength(2)
    expect(merged.find((r) => r.check.resource === 'nodes')?.allowed).toBe(false)
    expect(merged.find((r) => r.check.resource === 'secrets')?.allowed).toBe(true)
  })
})

describe('useMutationAccess', () => {
  const send = vi.fn()
  beforeEach(() => {
    send.mockReset()
    useStore.setState({ ...initialData(), send } as never)
  })

  it('is unknown and dispatches one check when nothing is cached', () => {
    const { result } = renderHook(() => useMutationAccess({ verb: 'update', resource: 'nodes' }))
    expect(result.current.status).toBe('unknown')
    expect(send).toHaveBeenCalledTimes(1)
    expect(deniedTitle(result.current, 'x')).toBeUndefined()
  })

  it('reports denied with the reason and does not re-dispatch', () => {
    useStore.setState({ rbacAccess: { selfAccess: [res('nodes', false, 'nope')] } } as never)
    const { result } = renderHook(() => useMutationAccess({ verb: 'update', resource: 'nodes' }))
    expect(result.current).toEqual({ status: 'denied', reason: 'nope' })
    expect(send).not.toHaveBeenCalled()
    expect(deniedTitle(result.current, 'cordon nodes')).toContain('nope')
  })

  it('reports allowed', () => {
    useStore.setState({ rbacAccess: { selfAccess: [res('nodes', true)] } } as never)
    const { result } = renderHook(() => useMutationAccess({ verb: 'update', resource: 'nodes' }))
    expect(result.current.status).toBe('allowed')
  })
})
