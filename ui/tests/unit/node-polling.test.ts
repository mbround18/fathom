import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { acquireNodeMetricsPolling, nodeUsageText } from '@/features/metrics/nodePolling'
import { useStore } from '@/store'

describe('node metrics polling', () => {
  const send = vi.fn()
  beforeEach(() => {
    vi.useFakeTimers()
    send.mockClear()
    useStore.setState({ send } as never)
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('shares one poll between holders and stops after the last release', () => {
    const a = acquireNodeMetricsPolling()
    const b = acquireNodeMetricsPolling()
    expect(send).toHaveBeenCalledTimes(1)
    expect(send).toHaveBeenCalledWith({
      type: 'StartNodeMetricsPolling',
      data: { interval_secs: 5 },
    })
    a()
    vi.advanceTimersByTime(5000)
    expect(send).toHaveBeenCalledTimes(1)
    b()
    b()
    vi.advanceTimersByTime(5000)
    expect(send).toHaveBeenLastCalledWith({ type: 'StopNodeMetricsPolling' })
    expect(send).toHaveBeenCalledTimes(2)
  })

  it('renders an em dash without metrics', () => {
    expect(nodeUsageText(undefined, 'cpu')).toBe('—')
  })
})
