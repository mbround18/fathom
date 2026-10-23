/**
 * "Image" tab (010-container-image-inspection, US1/US2/US3): on-demand OCI manifest/config
 * metadata for one container's image. Sends `InspectContainerImage` only when this tab is
 * mounted (never eagerly on Pod select, FR-003), and only again when the picked container or
 * the underlying pod changes — a cached result for the same container is not re-fetched.
 *
 * All new testids are prefixed `image-inspect-` (isolation from 008-pod-lifecycle-diagnostics'
 * `pod-lifecycle-` prefixed testids on the same Inspector page).
 */
import { useEffect, useState } from 'react'

import type { ImageInspectError, ImageInspection, PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, useStore } from '@/store'
import { imageInspectKey } from '@/store/handlers/imageInspect'

import { Picker } from '../Picker'
import { Empty, ErrorState, Loading } from '@companion/terminal'

const ERROR_TITLE: Record<ImageInspectError['kind'], string> = {
  RegistryUnreachable: 'Registry unreachable',
  ImageNotFound: 'Image not found',
  AuthenticationFailed: 'Authentication failed',
  AuthMethodUnsupported: 'Unsupported authentication method',
  Other: 'Inspection failed',
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(1)} ${units[unit]}`
}

export function ImageInspectPanel({ target }: { target: ObjectRef }) {
  const namespace = target.namespace ?? ''
  const pod = target.name
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const detailState = useStore((s) => s.details[detailKey(context, 'Pod', namespace, pod)])
  const detail =
    detailState?.eventType === 'PodDetailsUpdated' ? (detailState.data as PodDetails) : undefined
  const containers = detail?.containers ?? []

  const [selected, setSelected] = useState<string | undefined>(undefined)
  const container =
    containers.length > 0
      ? selected && containers.includes(selected)
        ? selected
        : containers[0]
      : undefined

  const key = container ? imageInspectKey(namespace, pod, container) : undefined
  const state = useStore((s) => (key ? s.imageInspections[key] : undefined))

  useEffect(() => {
    if (!container || !key) return
    // Re-opening this tab for a container whose result is already cached (ready or errored)
    // does not re-fetch — `useStore.getState()` here (not the reactive `state` above) so this
    // effect only depends on the identity of what's being inspected, not its fetch status.
    const cached = useStore.getState().imageInspections[key]
    if (cached && cached.status !== 'loading') return
    send({ type: 'InspectContainerImage', data: { namespace, pod, container } })
  }, [namespace, pod, container, key, send])

  if (!container) {
    return <Empty title="No containers" detail="This pod has no containers to inspect." />
  }

  return (
    <div
      className="flex h-full min-h-0 flex-col gap-3 overflow-auto p-3"
      data-testid="image-inspect-panel"
    >
      {containers.length > 1 && (
        <Picker
          label="Container"
          prefix="container"
          value={container}
          options={containers.map((c) => ({ value: c, label: c }))}
          onChange={setSelected}
        />
      )}
      {!state || state.status === 'loading' ? (
        <Loading rows={6} label="Inspecting image" />
      ) : state.status === 'error' && state.error ? (
        <ErrorState
          title={ERROR_TITLE[state.error.kind]}
          detail={
            <span data-testid={`image-inspect-error-${state.error.kind}`}>
              {state.error.detail}
            </span>
          }
        />
      ) : state.inspection ? (
        <ImageDetails inspection={state.inspection} />
      ) : null}
    </div>
  )
}

function ImageDetails({ inspection }: { inspection: ImageInspection }) {
  const { image_ref, total_size_bytes, layers, created, base_image } = inspection
  return (
    <div className="flex flex-col gap-4 text-sm">
      <section className="flex flex-col gap-1 font-mono text-xs" data-testid="image-inspect-ref">
        <p>
          <span className="text-muted-foreground">Registry </span>
          {image_ref.registry}
        </p>
        <p>
          <span className="text-muted-foreground">Repository </span>
          {image_ref.repository}
        </p>
        {image_ref.tag && (
          <p>
            <span className="text-muted-foreground">Tag </span>
            {image_ref.tag}
          </p>
        )}
        <p className="break-all">
          <span className="text-muted-foreground">Digest </span>
          {image_ref.digest}
        </p>
        <p>
          <span className="text-muted-foreground">Size </span>
          <span data-testid="image-inspect-size">{formatBytes(total_size_bytes)}</span>
        </p>
        {created && (
          <p>
            <span className="text-muted-foreground">Created </span>
            {created}
          </p>
        )}
      </section>

      <section
        className="flex flex-col gap-1 border-t border-border pt-2"
        data-testid="image-inspect-base-image"
      >
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Base image
        </h3>
        {base_image ? (
          <p className="text-xs text-muted-foreground">
            {base_image.label}{' '}
            <span className="text-muted-foreground">({base_image.confidence})</span>
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">Not determined</p>
        )}
      </section>

      <section
        className="flex flex-col gap-1 border-t border-border pt-2"
        data-testid="image-inspect-layers"
      >
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Layers ({layers.length})
        </h3>
        {layers.length === 0 ? (
          <p className="text-xs text-muted-foreground">No layers reported.</p>
        ) : (
          <ol className="flex flex-col gap-1">
            {layers.map((layer, i) => (
              <li
                key={`${layer.digest}-${String(i)}`}
                className="flex flex-col gap-0.5 border-b border-border/50 py-1 font-mono text-xs"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-muted-foreground">{layer.digest}</span>
                  <span className="shrink-0">{formatBytes(layer.size_bytes)}</span>
                </div>
                {layer.command && <p className="truncate text-foreground/80">{layer.command}</p>}
              </li>
            ))}
          </ol>
        )}
      </section>
    </div>
  )
}
