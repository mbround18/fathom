/**
 * `ImageInspectPanel` (010-container-image-inspection, US1/US2/US3): loading/success/error
 * rendering and the "send only on mount/container-change, never eagerly" contract (FR-003).
 * All testids asserted here are the `image-inspect-` prefixed ones this feature introduces.
 */
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ImageInspectPanel } from '@/components/panels/ImageInspectPanel'
import type { ImageInspection, PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, initialData, useStore } from '@/store'
import { imageInspectKey } from '@/store/handlers/imageInspect'

const TARGET: ObjectRef = { section: 'workloads/pods', kind: 'Pod', namespace: 'default', name: 'api-0' }

function podDetails(containers: string[]): PodDetails {
  return {
    namespace: 'default',
    name: 'api-0',
    creation_timestamp: '2024-01-01T00:00:00Z',
    labels: {},
    annotations: {},
    containers,
    ports: [],
    resource_specs: [],
    owner: null,
    resolved_owner: null,
    persistent_volume_claims: [],
    raw_json: '{}',
  }
}

const INSPECTION: ImageInspection = {
  image_ref: {
    registry: 'docker.io',
    repository: 'library/nginx',
    tag: 'latest',
    digest: 'sha256:abc123',
  },
  total_size_bytes: 5 * 1024 * 1024,
  layers: [
    {
      digest: 'sha256:layer1',
      size_bytes: 1024,
      media_type: 'application/vnd.oci.image.layer.v1.tar+gzip',
      command: 'COPY . .',
      empty_layer: false,
    },
  ],
  created: '2024-01-01T00:00:00Z',
  base_image: { label: 'nginx:1.25', confidence: 'high' },
}

function setPodDetails(containers: string[]) {
  useStore.setState((s) => ({
    details: {
      ...s.details,
      [detailKey(s.session.activeContext, 'Pod', 'default', 'api-0')]: {
        kind: 'Pod',
        eventType: 'PodDetailsUpdated',
        data: podDetails(containers),
        updatedAt: Date.now(),
      },
    },
  }))
}

beforeEach(() => {
  useStore.setState(initialData())
})

describe('ImageInspectPanel', () => {
  it('sends InspectContainerImage once PodDetails resolves a container, never eagerly before that', () => {
    const send = vi.fn()
    useStore.setState({ send })
    render(<ImageInspectPanel target={TARGET} />)
    expect(send).not.toHaveBeenCalled()

    setPodDetails(['app'])
    render(<ImageInspectPanel target={TARGET} />)
    expect(send).toHaveBeenCalledWith({
      type: 'InspectContainerImage',
      data: { namespace: 'default', pod: 'api-0', container: 'app' },
    })
  })

  it('shows a loading state while no result is cached yet', () => {
    useStore.setState({ send: vi.fn() })
    setPodDetails(['app'])
    render(<ImageInspectPanel target={TARGET} />)
    expect(screen.getByTestId('image-inspect-panel')).toBeInTheDocument()
    expect(screen.queryByTestId('image-inspect-ref')).toBeNull()
  })

  it('renders manifest details, layers, and base image on success', () => {
    useStore.setState({ send: vi.fn() })
    setPodDetails(['app'])
    useStore.setState((s) => ({
      imageInspections: {
        ...s.imageInspections,
        [imageInspectKey('default', 'api-0', 'app')]: {
          status: 'ready',
          inspection: INSPECTION,
          updatedAt: Date.now(),
        },
      },
    }))
    render(<ImageInspectPanel target={TARGET} />)
    expect(screen.getByTestId('image-inspect-ref')).toHaveTextContent('docker.io')
    expect(screen.getByTestId('image-inspect-ref')).toHaveTextContent('library/nginx')
    expect(screen.getByTestId('image-inspect-size')).toHaveTextContent('5.0 MiB')
    expect(screen.getByTestId('image-inspect-base-image')).toHaveTextContent('nginx:1.25')
    expect(screen.getByTestId('image-inspect-layers')).toHaveTextContent('sha256:layer1')
  })

  it('renders a distinct error state per ImageInspectError kind', () => {
    useStore.setState({ send: vi.fn() })
    setPodDetails(['app'])
    useStore.setState((s) => ({
      imageInspections: {
        ...s.imageInspections,
        [imageInspectKey('default', 'api-0', 'app')]: {
          status: 'error',
          error: { kind: 'AuthMethodUnsupported', detail: 'OIDC challenge not supported' },
          updatedAt: Date.now(),
        },
      },
    }))
    render(<ImageInspectPanel target={TARGET} />)
    expect(screen.getByTestId('image-inspect-error-AuthMethodUnsupported')).toHaveTextContent(
      'OIDC challenge not supported',
    )
  })

  it('shows an empty state when the pod has no containers', () => {
    useStore.setState({ send: vi.fn() })
    setPodDetails([])
    render(<ImageInspectPanel target={TARGET} />)
    expect(screen.getAllByText(/no containers/i).length).toBeGreaterThan(0)
  })
})
