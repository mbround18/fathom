/**
 * Action menus (T095): the pane-header menu (list selection or inspector target), the row menu
 * (right-click, long-press on touch, Shift+F10/ContextMenu key, or the row's ⋯ button), and the
 * selection bar for bulk actions. Every entry only opens a dialog (`store.openAction`); nothing
 * is sent from here.
 *
 * Test hooks: `list-actions`, `inspector-actions`, `row-actions` (triggers), `row-action-menu`,
 * `action-delete` / `action-scale` / `action-restart` (menu items), `selection-bar`,
 * `selection-count`, `bulk-delete` / `bulk-restart` / `bulk-scale`, `selection-clear`.
 */
import {
  ArrowClockwiseIcon,
  ArrowsOutLineVerticalIcon,
  BugIcon,
  LightningIcon,
  PauseIcon,
  PlayIcon,
  DotsThreeVerticalIcon,
  TrashIcon,
  XIcon,
  type Icon,
} from '@phosphor-icons/react'
import { useMemo, useRef } from 'react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Gated } from '@/lib/capabilities'
import { cn } from '@/lib/utils'
import {
  descriptorFor,
  type ActionDescriptor,
  type AnyDescriptor,
  type ObjectRef,
} from '@/resources'
import { actionsFor } from '@/resources/actions'
import { objectName, summarize } from '@/store/actions'
import { listKey, useStore, type ListRow, type PaneId, type PaneState } from '@/store'

import { actionTarget, promptAction, useCheckedRows } from './action-utils'

const ICON: Record<ActionDescriptor<unknown>['id'], Icon> = {
  delete: TrashIcon,
  scale: ArrowsOutLineVerticalIcon,
  restart: ArrowClockwiseIcon,
  debug: BugIcon,
  suspend: PauseIcon,
  resume: PlayIcon,
  trigger: LightningIcon,
}

type AnyAction = ActionDescriptor<ListRow>

function ActionItems({
  descriptor,
  rows,
  onPick,
}: {
  descriptor: AnyDescriptor
  rows: readonly ListRow[]
  onPick: (a: AnyAction) => void
}) {
  const actions = actionsFor(descriptor.actions, rows.length)
  if (actions.length === 0) {
    return <DropdownMenuLabel>No actions for {rows.length} objects</DropdownMenuLabel>
  }
  return actions.map((a, i) => {
    const IconCmp = ICON[a.id]
    return (
      <div key={a.id} className="contents">
        {a.destructive && i > 0 && <DropdownMenuSeparator />}
        <Gated {...actionTarget(descriptor, a, rows)}>
          <DropdownMenuItem
            data-testid={`action-${a.id}`}
            variant={a.destructive ? 'destructive' : 'default'}
            onSelect={() => onPick(a)}
            className="min-h-[max(2rem,var(--touch-target))] font-display text-xs font-semibold tracking-wider uppercase"
          >
            <IconCmp aria-hidden="true" />
            {a.label}
            {rows.length > 1 ? ` ${String(rows.length)}` : ''}…
            {a.id === 'delete' && (
              <DropdownMenuShortcut aria-hidden="true">Del</DropdownMenuShortcut>
            )}
          </DropdownMenuItem>
        </Gated>
      </div>
    )
  })
}

function MenuHeading({
  descriptor,
  rows,
}: {
  descriptor: AnyDescriptor
  rows: readonly ListRow[]
}) {
  const [only] = rows
  return (
    <DropdownMenuLabel className="max-w-72 font-mono break-all text-muted-foreground">
      {rows.length === 1 && only !== undefined ? (
        <>
          <span className="text-accent-neon">{descriptor.kind}</span> {objectName(descriptor, only)}
        </>
      ) : (
        summarize(descriptor, rows)
      )}
    </DropdownMenuLabel>
  )
}

interface PaneActionMenuProps {
  descriptor: AnyDescriptor
  rows: readonly ListRow[]
  paneId?: PaneId
  testId: string
  className?: string
}

/** The ⋮ menu in a pane header. */
export function PaneActionMenu({
  descriptor,
  rows,
  paneId,
  testId,
  className,
}: PaneActionMenuProps) {
  const trigger = useRef<HTMLButtonElement>(null)
  const picked = useRef(false)
  const available = actionsFor(descriptor.actions, rows.length).length > 0
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          ref={trigger}
          variant="ghost"
          size="icon"
          aria-label="Actions"
          title={
            rows.length > 0 ? `Actions for ${summarize(descriptor, rows)}` : 'Select a row first'
          }
          data-testid={testId}
          disabled={!available}
          className={cn(
            'size-[max(2rem,var(--touch-target))] shrink-0 text-muted-foreground hover:text-accent-neon',
            className,
          )}
        >
          <DotsThreeVerticalIcon aria-hidden="true" weight="bold" className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-auto min-w-48"
        onCloseAutoFocus={(e) => {
          // The dialog that just opened owns focus (and returns it to this trigger).
          if (picked.current) e.preventDefault()
          picked.current = false
        }}
      >
        <MenuHeading descriptor={descriptor} rows={rows} />
        <DropdownMenuSeparator />
        <ActionItems
          descriptor={descriptor}
          rows={rows}
          onPick={(a) => {
            picked.current = true
            promptAction(descriptor, a, rows, {
              ...(paneId !== undefined ? { paneId } : {}),
              returnFocus: trigger.current,
            })
          }}
        />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export interface RowMenuState {
  rowKey: string
  x: number
  y: number
  /** Long-press: the full text of the pressed cell (the touch "peek", FR-010). */
  peek?: string
}

interface RowActionMenuProps {
  descriptor: AnyDescriptor
  row: ListRow
  menu: RowMenuState
  paneId: PaneId
  /** Where focus returns when the menu closes without an action (the grid). */
  returnFocus: HTMLElement | null
  onClose: () => void
}

/** Context menu for one row, anchored where it was opened. */
export function RowActionMenu({
  descriptor,
  row,
  menu,
  paneId,
  returnFocus,
  onClose,
}: RowActionMenuProps) {
  const picked = useRef(false)
  const rows = useMemo(() => [row], [row])
  return (
    <DropdownMenu
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DropdownMenuTrigger asChild>
        <span
          aria-hidden="true"
          className="pointer-events-none fixed size-0"
          style={{ left: menu.x, top: menu.y }}
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        data-testid="row-action-menu"
        align="start"
        className="w-auto max-w-[min(20rem,calc(100vw-2rem))] min-w-48"
        onCloseAutoFocus={(e) => {
          e.preventDefault()
          if (!picked.current) returnFocus?.focus()
        }}
      >
        <MenuHeading descriptor={descriptor} rows={rows} />
        {menu.peek !== undefined && menu.peek !== descriptor.ref(row).name && (
          <p
            data-testid="row-peek"
            className="px-2 pb-2 font-mono text-xs break-all text-foreground"
          >
            {menu.peek}
          </p>
        )}
        <DropdownMenuSeparator />
        <ActionItems
          descriptor={descriptor}
          rows={rows}
          onPick={(a) => {
            picked.current = true
            promptAction(descriptor, a, rows, { paneId, returnFocus })
          }}
        />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Bulk bar under a list's search while rows are checked. */
export function SelectionBar({
  pane,
  descriptor,
  rows,
}: {
  pane: PaneState
  descriptor: AnyDescriptor
  rows: readonly ListRow[]
}) {
  const checked = useCheckedRows(pane.id, descriptor, rows)
  const setSelection = useStore((s) => s.setSelection)
  if (checked.length === 0) return null
  const actions = actionsFor(descriptor.actions, checked.length)
  const focusGrid = () =>
    document.querySelector<HTMLElement>(`[data-pane-id="${pane.id}"] [role="grid"]`)

  return (
    <div
      role="toolbar"
      aria-label={`${String(checked.length)} selected`}
      data-testid="selection-bar"
      className="flex shrink-0 items-center gap-1 border-b border-accent-neon/60 bg-secondary/70 pl-2 text-xs"
    >
      <span data-testid="selection-count" className="mr-auto truncate font-mono">
        <span aria-hidden="true" className="text-accent-neon">
          ✓{' '}
        </span>
        {checked.length} selected
      </span>
      {actions.map((a) => {
        const IconCmp = ICON[a.id]
        return (
          <Gated key={a.id} {...actionTarget(descriptor, a, checked)}>
            <Button
              variant={a.destructive ? 'destructive' : 'ghost'}
              data-testid={`bulk-${a.id}`}
              onClick={(e) =>
                promptAction(descriptor, a, checked, {
                  paneId: pane.id,
                  returnFocus: focusGrid() ?? e.currentTarget,
                })
              }
              className="h-[max(2rem,var(--touch-target))] px-2.5"
            >
              <IconCmp aria-hidden="true" data-icon="inline-start" />
              {a.label}
              {checked.length > 1 ? ` ${String(checked.length)}` : ''}
            </Button>
          </Gated>
        )
      })}
      <Button
        variant="ghost"
        size="icon"
        aria-label="Clear selection"
        data-testid="selection-clear"
        onClick={() => {
          setSelection(pane.id, [])
          focusGrid()?.focus()
        }}
        className="size-[max(2rem,var(--touch-target))]"
      >
        <XIcon aria-hidden="true" />
      </Button>
    </div>
  )
}

const NO_ROWS: readonly ListRow[] = []

/** The list pane header's menu: checked rows, else the row open in the inspector. */
export function ListActions({ pane }: { pane: PaneState }) {
  const section = pane.route?.section
  const descriptor = descriptorFor(section)
  const ctx = useStore((s) => s.session.activeContext)
  const scope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const rows = useStore((s) =>
    section === undefined
      ? NO_ROWS
      : (s.lists[listKey(ctx, section, scope, null)]?.rows ?? NO_ROWS),
  )
  const checked = useCheckedRows(pane.id, descriptor, rows)
  const target = useMemo(() => {
    if (checked.length > 0 || !descriptor || pane.selection === undefined) return checked
    const open = rows.find((r) => descriptor.rowKey(r) === pane.selection)
    return open ? [open] : []
  }, [checked, descriptor, pane.selection, rows])
  if (!descriptor || descriptor.actions.length === 0) return null
  return (
    <PaneActionMenu descriptor={descriptor} rows={target} paneId={pane.id} testId="list-actions" />
  )
}

/** The inspector header's menu, for the object it shows (while it is still in its list). */
export function InspectorActions({ target, source }: { target: ObjectRef; source?: PaneId }) {
  const descriptor = descriptorFor(target.section)
  const ctx = useStore((s) => s.session.activeContext)
  const listScope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const list = useStore((s) => s.lists[listKey(ctx, target.section, listScope, null)])
  const rowKey = target.namespace ? `${target.namespace}/${target.name}` : target.name
  const rows = useMemo(() => {
    const row = descriptor
      ? (list?.rows ?? NO_ROWS).find((r) => descriptor.rowKey(r) === rowKey)
      : undefined
    return row ? [row] : []
  }, [descriptor, list, rowKey])
  if (!descriptor || descriptor.actions.length === 0) return null
  return (
    <PaneActionMenu
      descriptor={descriptor}
      rows={rows}
      {...(source !== undefined ? { paneId: source } : {})}
      testId="inspector-actions"
    />
  )
}
