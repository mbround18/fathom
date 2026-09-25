/**
 * 008-pod-lifecycle-diagnostics: frontend-only correlation between a container's probe/hook
 * *configuration* (`ContainerLifecycle.probes`/`hooks`, from the backend) and its currently
 * *failing* state, derived from the Pod's own Events (already fetched via `RefreshEvents` —
 * research.md's "frontend-side probe/hook failure correlation" decision; no new protocol type).
 *
 * Matching rules (research.md):
 * - Readiness probe: failing when the container is currently running but not ready (the
 *   kubelet-computed state already reflects probe failures), OR a matching `Unhealthy` event.
 * - Liveness/startup probes: failing only when a matching `Unhealthy` event mentions the
 *   container and the corresponding "Liveness probe failed" / "Startup probe failed" text — the
 *   Pod's `ready`/`state` fields don't carry liveness/startup outcomes.
 * - Lifecycle hooks: failing when a `FailedPostStartHook` / `FailedPreStopHook` event mentions
 *   the container.
 *
 * All matching is a case-insensitive substring match on the event's `message` (research.md:
 * events don't carry a structured container reference), which is best-effort by design — an event
 * that doesn't mention the container by name won't be attributed to it.
 */
import type {
  ContainerLifecycle,
  KubernetesEvent,
  LifecycleHookSpec,
  ProbeSpec,
} from '@/protocol/types'

export interface ProbeStateEntry {
  probe: ProbeSpec
  failing: boolean
}

export interface HookStateEntry {
  hook: LifecycleHookSpec
  failed: boolean
}

function mentionsContainer(message: string, containerName: string): boolean {
  return message.toLowerCase().includes(containerName.toLowerCase())
}

function hasMatchingEvent(
  events: readonly KubernetesEvent[],
  containerName: string,
  reason: string,
  messageSubstring: string,
): boolean {
  return events.some(
    (e) =>
      e.reason === reason &&
      e.message.toLowerCase().includes(messageSubstring.toLowerCase()) &&
      mentionsContainer(e.message, containerName),
  )
}

/** Per-container probe failure state (US2, FR-007). */
export function probeState(
  container: ContainerLifecycle,
  events: readonly KubernetesEvent[],
): ProbeStateEntry[] {
  return container.probes.map((probe) => {
    let failing = false
    if (probe.probe_type === 'readiness') {
      failing =
        (container.state.phase === 'running' && !container.ready) ||
        hasMatchingEvent(events, container.name, 'Unhealthy', 'readiness probe failed')
    } else if (probe.probe_type === 'liveness') {
      failing = hasMatchingEvent(events, container.name, 'Unhealthy', 'liveness probe failed')
    } else if (probe.probe_type === 'startup') {
      failing = hasMatchingEvent(events, container.name, 'Unhealthy', 'startup probe failed')
    }
    return { probe, failing }
  })
}

/** Per-container lifecycle hook failure state (US4, FR-012). */
export function hookState(
  container: ContainerLifecycle,
  events: readonly KubernetesEvent[],
): HookStateEntry[] {
  return container.hooks.map((hook) => {
    const reason = hook.hook_type === 'postStart' ? 'FailedPostStartHook' : 'FailedPreStopHook'
    const failed = events.some(
      (e) => e.reason === reason && mentionsContainer(e.message, container.name),
    )
    return { hook, failed }
  })
}
