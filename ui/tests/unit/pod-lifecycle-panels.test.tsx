/**
 * 008-pod-lifecycle-diagnostics: component-level coverage for `PodLifecyclePanels`
 * (T009 — init containers, T021 — restart history, T031 — events timeline). Probe/hook failure
 * correlation itself is covered by `pod-lifecycle.test.ts`; this file checks the section renders
 * the right rows/empty-states from a `containers_lifecycle` fixture and reacts to a live
 * `EventsUpdated` batch (US5's "in-place" requirement).
 */
import { render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// `@/store` must be imported before any descriptor module (see ingest.test.ts's note).
import { initialData, useStore } from '@/store'
import { ingest } from '@/store/ingest'
import { PodLifecyclePanels } from '@/components/PodLifecyclePanels'

import { DETAIL_POD, podLifecycleEvents, twoContexts } from '../fixtures'
import { batch, clusterRoute, list } from '../fixtures/builders'
import { podDetailsWithLifecycle, podEventsWithLifecycle } from '../fixtures/gen-pods'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

beforeEach(() => {
  useStore.setState(initialData())
  ingest(twoContexts)
})

function renderPanels() {
  return render(
    <PodLifecyclePanels namespace={DETAIL_POD.namespace} name={DETAIL_POD.name} containers={[]} />,
  )
}

describe('PodLifecyclePanels', () => {
  it('lists init containers separately from app containers, with exit codes, and an empty probes/hooks state for containers with none', () => {
    ingest(podLifecycleEvents(DETAIL_POD))
    const { containers_lifecycle: containers } = podDetailsWithLifecycle(DETAIL_POD)
    render(
      <PodLifecyclePanels
        namespace={DETAIL_POD.namespace}
        name={DETAIL_POD.name}
        containers={containers}
      />,
    )

    const init = screen.getByTestId('pod-lifecycle-init-containers')
    expect(within(init).getByTestId('pod-lifecycle-init-container-init-migrate')).toHaveTextContent(
      'exit 0',
    )
    expect(within(init).getByTestId('pod-lifecycle-init-container-init-verify')).toHaveTextContent(
      'exit 1',
    )
    // The app container (in `containers`, not `containers_lifecycle`'s init subset) is not listed here.
    expect(within(init).queryByText(DETAIL_POD.name)).toBeNull()

    // The sidecar has no probes/hooks configured -> its per-container empty states render.
    expect(screen.getAllByTestId('pod-lifecycle-probes-empty').length).toBeGreaterThan(0)
    expect(screen.getAllByTestId('pod-lifecycle-hooks-empty').length).toBeGreaterThan(0)
  })

  it('shows an empty state when the pod has no init containers', () => {
    renderPanels()
    expect(screen.getByTestId('pod-lifecycle-init-containers-empty')).toBeInTheDocument()
  })

  it('falls back to `state` for restart history when restartPolicy: Never leaves last_state unset', () => {
    const container = {
      name: 'batch-job',
      is_init: false,
      is_ephemeral: false,
      image: 'registry.example.com/batch:1.0.0',
      ready: false,
      state: {
        phase: 'terminated' as const,
        reason: 'Error',
        exit_code: 1,
        started_at: '2026-09-20T08:00:00Z',
        finished_at: '2026-09-20T08:00:05Z',
      },
      restart_count: 0,
      last_state: null,
      probes: [],
      hooks: [],
    }
    render(
      <PodLifecyclePanels
        namespace={DETAIL_POD.namespace}
        name={DETAIL_POD.name}
        containers={[container]}
      />,
    )
    const restarts = screen.getByTestId('pod-lifecycle-restarts')
    expect(restarts).toHaveTextContent('Error')
    expect(restarts).toHaveTextContent('exit 1')
  })

  it('renders the pod Events timeline, including an event with an unrecognized reason, and updates live', async () => {
    renderPanels()
    expect(screen.getByTestId('pod-lifecycle-events-timeline-empty')).toBeInTheDocument()

    ingest(podLifecycleEvents(DETAIL_POD))
    const timeline = await screen.findByTestId('pod-lifecycle-events-timeline')
    expect(timeline).toHaveTextContent('SomeUnknownReason')

    // A second EventsUpdated batch (e.g. a new BackOff) reflects live without remount.
    const more = [
      ...podEventsWithLifecycle(DETAIL_POD),
      {
        namespace: DETAIL_POD.namespace,
        involved_object_kind: 'Pod' as const,
        involved_object_name: DETAIL_POD.name,
        name: `${DETAIL_POD.name}.late`,
        event_type: 'Warning' as const,
        reason: 'BackOff',
        message: 'Back-off restarting failed container',
        count: 1,
        first_timestamp: '2026-09-20T09:00:00Z',
        last_timestamp: '2026-09-20T09:00:00Z',
        source: 'kubelet',
      },
    ]
    ingest(
      batch([
        list(60, clusterRoute('kind-a', 'events'), {
          type: 'EventsUpdated',
          data: { events: more, total_count: more.length },
        }),
      ]),
    )
    await expect
      .poll(() => screen.getByTestId('pod-lifecycle-events-timeline').textContent)
      .toContain('BackOff')
  })
})
