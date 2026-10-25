import { DotsSixVerticalIcon } from '@phosphor-icons/react'
import type { DragEvent } from 'react'

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@companion/ui'
import type { PaneId } from '@/store'

import { listSlotTargets, swapListPanes } from './pane-actions'

const MIME = 'application/x-fathom-pane'

/** Drag handlers making a list pane's header a swap source and target (HTML5 DnD). */
export function paneDragProps(paneId: PaneId) {
  return {
    draggable: true,
    onDragStart: (e: DragEvent) => {
      e.dataTransfer.setData(MIME, paneId)
      e.dataTransfer.effectAllowed = 'move'
    },
    onDragOver: (e: DragEvent) => {
      if (e.dataTransfer.types.includes(MIME)) e.preventDefault()
    },
    onDrop: (e: DragEvent) => {
      const from = e.dataTransfer.getData(MIME)
      if (from) {
        e.preventDefault()
        swapListPanes(from, paneId)
      }
    },
  }
}

/** "Pin to slot" menu for a list pane: exchange this pane's view with another list slot. */
export function PaneSlotMenu({ paneId }: { paneId: PaneId }) {
  const targets = listSlotTargets(paneId)
  if (targets.length === 0) return null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Pane slot menu"
          data-testid="pane-slot-menu"
          className="size-[max(2rem,var(--touch-target))] shrink-0 text-muted-foreground hover:text-accent-neon"
        >
          <DotsSixVerticalIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Pin to slot</DropdownMenuLabel>
        {targets.map(([id, i]) => (
          <DropdownMenuItem key={id} onSelect={() => swapListPanes(paneId, id)}>
            Pin to slot {i + 1}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
