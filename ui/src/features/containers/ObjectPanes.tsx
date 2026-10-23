/**
 * 048-docker-management US3: the images, volumes and networks this computer is holding.
 *
 * These three panes answer one question — *what is safe to delete* — so they are one component
 * with one shape rather than three that drift apart. Every row carries the three facts the answer
 * needs: how much space it is holding, whether anything still references it, and which project it
 * belongs to. A size the runtime did not report reads as "size not reported", never as `0 B`:
 * "unavailable" and "empty" are different claims and only one of them is true.
 *
 * Read-only unless adopted, exactly as the container verbs are (FR-012–FR-017). An object whose
 * project has not been adopted has no Remove button at all, because the backend would exclude it
 * from the removal anyway and a button that cannot work is worse than no button. An object no
 * project owns is the operator's to remove: there is no other project whose work it could be.
 *
 * Prune is offered as a *selection*, not as a prune. The button takes a preview, and what that
 * preview contains is the identities the filter covers right now (FR-026) — so what the operator
 * approves is a list they have read, and the daemon cannot add to it in between.
 */
import { useState } from 'react'

import { Button } from '@companion/ui'
import { cn } from '@companion/theme'
import type { ObjectClass, PruneFilter, RemovableObject } from '@/protocol/types'
import { useStore } from '@/store'

import { CreateObjectForm, createLabel } from './CreateForms'
import { OperationPreview } from './OperationPreview'
import { sizeLabel } from './labels'

/** The pane's own words for each class, so nothing is labelled "object". */
const COPY: Record<
  Exclude<ObjectClass, 'container'>,
  { title: string; empty: string; prune: { filter: PruneFilter; label: string }[] }
> = {
  image: {
    title: 'Images',
    empty: 'The runtime is holding no images.',
    prune: [
      { filter: 'dangling_images', label: 'Select untagged images' },
      { filter: 'unused_images', label: 'Select images nothing uses' },
    ],
  },
  volume: {
    title: 'Volumes',
    empty: 'The runtime is holding no volumes.',
    prune: [{ filter: 'unused_volumes', label: 'Select volumes nothing holds' }],
  },
  network: {
    title: 'Networks',
    empty: 'The runtime has no networks of its own.',
    prune: [{ filter: 'unused_networks', label: 'Select networks nothing joins' }],
  },
}

/**
 * How an image's tag state reads.
 *
 * Three words, not two. `null` means the runtime did not say, and calling that "untagged" would
 * invite a prune of an image that may well be tagged.
 */
function tagWord(tagged: boolean | null): string {
  if (tagged === true) return 'tagged'
  if (tagged === false) return 'untagged'
  return 'not classified'
}

/** What still holds this object, in the words of the things holding it. */
function referenceWords(object: RemovableObject): string {
  if (object.referenced_by.length === 0) return 'nothing uses it'
  return object.referenced_by
    .map((r) => {
      if (r.kind === 'cluster') return `cluster ${r.name}`
      if (r.kind === 'running_container') return `${r.name} (running)`
      if (r.kind === 'stopped_container') return `${r.name} (stopped)`
      return r.name
    })
    .join(', ')
}

function Row({
  object,
  adopted,
  onRemove,
}: {
  object: RemovableObject
  adopted: boolean
  onRemove: () => void
}) {
  const size = sizeLabel(object.size_bytes)
  const inUse = object.referenced_by.length > 0
  return (
    <li
      data-testid={`object-row-${object.id}`}
      data-in-use={inUse ? 'true' : 'false'}
      className="flex min-w-0 flex-col gap-1 border-b border-border/60 px-3 py-2 last:border-b-0 sm:flex-row sm:items-baseline sm:gap-3"
    >
      <span className="min-w-0 flex-1 truncate font-mono text-xs">
        {object.name === '' ? object.id : object.name}
      </span>
      {object.class === 'image' && (
        <span
          data-testid={`object-tagged-${object.id}`}
          className="shrink-0 text-[10px] tracking-wider text-muted-foreground uppercase"
        >
          {tagWord(object.tagged)}
        </span>
      )}
      <span
        data-testid={`object-references-${object.id}`}
        className={cn(
          'min-w-0 truncate text-xs',
          inUse ? 'text-foreground' : 'text-muted-foreground',
        )}
      >
        {referenceWords(object)}
      </span>
      {object.project !== null && (
        <span className="shrink-0 text-xs text-muted-foreground">{object.project}</span>
      )}
      <span
        data-testid={`object-size-${object.id}`}
        className="shrink-0 text-xs text-muted-foreground"
      >
        {size ?? 'size not reported'}
      </span>
      {adopted ? (
        <Button
          size="sm"
          variant="destructive"
          data-testid={`object-remove-${object.id}`}
          onClick={onRemove}
        >
          Remove
        </Button>
      ) : (
        /* Not a disabled button: the reason is the useful part, and a greyed-out Remove makes the
           operator hunt for why. The grant lives on the project, in the Containers pane. */
        <span
          data-testid={`object-locked-${object.id}`}
          className="shrink-0 text-xs text-muted-foreground"
        >
          {object.project ?? 'its project'} is not adopted
        </span>
      )}
    </li>
  )
}

export function ObjectPane({ objectClass }: { objectClass: Exclude<ObjectClass, 'container'> }) {
  const inventory = useStore((s) => s.containerRuntime.inventory)
  const granted = useStore((s) => s.containerRuntime.adopted)
  const preview = useStore((s) => s.previewContainerOperation)
  const operation = useStore((s) => s.containerRuntime.operation)
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)

  const copy = COPY[objectClass]
  if (!inventory) return null
  const objects =
    objectClass === 'image'
      ? inventory.images
      : objectClass === 'volume'
        ? inventory.volumes
        : inventory.networks

  // A second preview while one is in flight would leave two argvs on screen and one confirmation
  // field between them (FR-029).
  const busy = operation !== undefined
  const total = sizeLabel(objects.reduce((sum, o) => sum + (o.size_bytes ?? 0), 0))
  // Only when something reported a size at all. A total of `0 B` over objects whose sizes are all
  // unknown would be a number Fathom made up.
  const anySized = objects.some((o) => o.size_bytes !== null)

  return (
    <div
      data-testid={`object-pane-${objectClass}`}
      className="flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-2">
        <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {objects.length === 0
            ? copy.empty
            : `${String(objects.length)} ${copy.title.toLowerCase()}`}
          {anySized && total !== null && (
            <span data-testid={`object-total-${objectClass}`}> · {total} held</span>
          )}
        </p>
        <Button
          size="sm"
          variant={creating ? 'default' : 'ghost'}
          data-testid={`object-create-${objectClass}`}
          disabled={busy}
          onClick={() => {
            setCreating((c) => !c)
          }}
        >
          {createLabel(objectClass)}
        </Button>
        {copy.prune.map((p) => (
          <Button
            key={p.filter}
            size="sm"
            variant="ghost"
            data-testid={`object-prune-${p.filter}`}
            disabled={busy || objects.length === 0}
            onClick={() => {
              setSelected(null)
              preview({ kind: 'prune_selection', class: objectClass, filter: p.filter })
            }}
          >
            {p.label}
          </Button>
        ))}
      </div>

      <ul className="min-h-0 flex-1 overflow-y-auto">
        {objects.map((object) => (
          <Row
            key={object.id}
            object={object}
            adopted={object.project === null || granted.includes(object.project)}
            onRemove={() => {
              if (busy) return
              setSelected(object.id)
              preview({
                kind: 'remove',
                objects: [{ class: object.class, id: object.id, name: object.name }],
              })
            }}
          />
        ))}
      </ul>

      {/* Creation is form-driven and nothing more: the form ends at a preview, so the operator's
          last act is still approving an argv (FR-019). */}
      {creating && operation === undefined && (
        <CreateObjectForm
          objectClass={objectClass}
          onDone={() => {
            setCreating(false)
          }}
        />
      )}

      {operation !== undefined && (
        <div className="shrink-0 border-t border-border p-3" data-selected={selected ?? ''}>
          <OperationPreview />
        </div>
      )}
    </div>
  )
}
