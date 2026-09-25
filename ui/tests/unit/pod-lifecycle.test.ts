/**
 * 008-pod-lifecycle-diagnostics: unit coverage for the frontend-only probe/hook failure
 * correlation logic in `ui/src/resources/podLifecycle.ts` (T015, T026) — the matching rules
 * described there: readiness derives from `state`/`ready` OR a matching `Unhealthy` event;
 * liveness/startup and hooks derive solely from matching events; matching is a case-insensitive
 * substring match on `message` scoped to the specific container name.
 */
import { describe, expect, it } from 'vitest'

import type { ContainerLifecycle, KubernetesEvent } from '@/protocol/types'
import { hookState, probeState } from '@/resources/podLifecycle'

function container(overrides: Partial<ContainerLifecycle> = {}): ContainerLifecycle {
  return {
    name: 'api',
    is_init: false,
    is_ephemeral: false,
    image: 'registry.example.com/api:1.0.0',
    ready: true,
    state: { phase: 'running', reason: null, exit_code: null, started_at: null, finished_at: null },
    restart_count: 0,
    last_state: null,
    probes: [],
    hooks: [],
    ...overrides,
  }
}

function event(overrides: Partial<KubernetesEvent> = {}): KubernetesEvent {
  return {
    name: 'api.1',
    namespace: 'default',
    involved_object_kind: 'Pod',
    involved_object_name: 'api-abc',
    event_type: 'Warning',
    reason: 'Unhealthy',
    message: 'Liveness probe failed: HTTP probe failed with statuscode: 503 (container api)',
    count: 1,
    first_timestamp: '2026-09-20T08:15:00Z',
    last_timestamp: '2026-09-20T08:15:00Z',
    source: 'kubelet',
    ...overrides,
  }
}

describe('probeState', () => {
  it('readiness probe fails when the container is running but not ready, with no event needed', () => {
    const c = container({
      ready: false,
      probes: [
        {
          probe_type: 'readiness',
          mechanism: 'httpGet',
          target: 'GET /healthz:8080',
          period_seconds: 10,
          failure_threshold: 3,
          success_threshold: 1,
        },
      ],
    })
    const [entry] = probeState(c, [])
    expect(entry?.failing).toBe(true)
  })

  it('readiness probe is not failing when the container is running and ready', () => {
    const c = container({
      ready: true,
      probes: [
        {
          probe_type: 'readiness',
          mechanism: 'httpGet',
          target: 'GET /healthz:8080',
          period_seconds: 10,
          failure_threshold: 3,
          success_threshold: 1,
        },
      ],
    })
    const [entry] = probeState(c, [])
    expect(entry?.failing).toBe(false)
  })

  it('liveness probe fails only via a matching Unhealthy event mentioning the container', () => {
    const c = container({
      probes: [
        {
          probe_type: 'liveness',
          mechanism: 'httpGet',
          target: 'GET /livez:8080',
          period_seconds: 15,
          failure_threshold: 3,
          success_threshold: 1,
        },
      ],
    })
    const events = [
      event({ message: 'Liveness probe failed: HTTP probe failed with statuscode: 503 (container api)' }),
    ]
    const [entry] = probeState(c, events)
    expect(entry?.failing).toBe(true)
  })

  it('liveness probe is not failing when no matching event exists', () => {
    const c = container({
      probes: [
        {
          probe_type: 'liveness',
          mechanism: 'httpGet',
          target: 'GET /livez:8080',
          period_seconds: 15,
          failure_threshold: 3,
          success_threshold: 1,
        },
      ],
    })
    const [entry] = probeState(c, [])
    expect(entry?.failing).toBe(false)
  })

  it('startup probe fails only via a matching Unhealthy event, scoped to the right container', () => {
    const c = container({
      name: 'worker',
      probes: [
        {
          probe_type: 'startup',
          mechanism: 'tcpSocket',
          target: 'tcp:8080',
          period_seconds: 5,
          failure_threshold: 30,
          success_threshold: 1,
        },
      ],
    })
    const wrongContainer = [
      event({
        reason: 'Unhealthy',
        message: 'Startup probe failed: dial tcp (container other)',
      }),
    ]
    expect(probeState(c, wrongContainer)[0]?.failing).toBe(false)

    const rightContainer = [
      event({
        reason: 'Unhealthy',
        message: 'Startup probe failed: dial tcp (container worker)',
      }),
    ]
    expect(probeState(c, rightContainer)[0]?.failing).toBe(true)
  })

  it('returns an entry per configured probe, empty when none configured', () => {
    expect(probeState(container(), [])).toEqual([])
  })
})

describe('hookState', () => {
  it('postStart hook is failed only via a matching FailedPostStartHook event', () => {
    const c = container({
      hooks: [{ hook_type: 'postStart', mechanism: 'exec', target: 'exec: /bin/sh -c warm-cache.sh' }],
    })
    const events = [
      event({
        reason: 'FailedPostStartHook',
        message: 'Exec lifecycle hook (...) for Container "api" in Pod "api-abc_default(uid)" failed',
      }),
    ]
    expect(hookState(c, events)[0]?.failed).toBe(true)
  })

  it('preStop hook is failed only via a matching FailedPreStopHook event', () => {
    const c = container({
      hooks: [{ hook_type: 'preStop', mechanism: 'httpGet', target: 'GET /shutdown:8080' }],
    })
    expect(hookState(c, [])[0]?.failed).toBe(false)
    const events = [
      event({
        reason: 'FailedPreStopHook',
        message: 'PreStopHook failed for Container "api" in Pod "api-abc_default(uid)"',
      }),
    ]
    expect(hookState(c, events)[0]?.failed).toBe(true)
  })

  it('does not attribute a hook-failure event to a container it does not mention', () => {
    const c = container({
      name: 'sidecar',
      hooks: [{ hook_type: 'postStart', mechanism: 'exec', target: 'exec: /bin/sh -c warm.sh' }],
    })
    const events = [
      event({ reason: 'FailedPostStartHook', message: 'failed for Container "api" in Pod "x"' }),
    ]
    expect(hookState(c, events)[0]?.failed).toBe(false)
  })
})
