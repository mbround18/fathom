/**
 * A pane's tab strip: one tab per stream, the active one marked, each closable; and, for the
 * active tab, moving it within the strip, to a neighbouring pane, or into a new pane (split).
 * Every control is a button, so the layout is arranged from the keyboard as well as the pointer.
 */
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CaretLeftIcon,
  CaretRightIcon,
  ChartBarIcon,
  ScrollIcon,
  SquareSplitHorizontalIcon,
  TerminalWindowIcon,
  XIcon,
} from '@phosphor-icons/react'
import { cn } from '@companion/theme'
import { Button } from '@companion/ui'

import { MAX_PANES, tabTitle, type TabSpec } from './layout'
import { useTabs } from './store'

const KIND_ICON = { logs: ScrollIcon, shell: TerminalWindowIcon, stats: ChartBarIcon } as const
const KIND_LABEL = { logs: 'Logs', shell: 'Shell', stats: 'Stats' } as const

export interface TabStripProps {
  paneId: string
  /** The pane's place among the panes; `null` at narrow widths, where there is one strip. */
  index: number | null
  paneCount: number
  tabs: TabSpec[]
  active: string | null
}

export function TabStrip({ paneId, index, paneCount, tabs, active }: TabStripProps) {
  const { focusTab, closeTab, moveTab, reorderTab } = useTabs.getState()
  const current = tabs.find((t) => t.id === active)
  const at = current ? tabs.indexOf(current) : -1

  return (
    <div className="flex min-w-0 items-stretch border-b border-border bg-card">
      <div role="tablist" aria-label="Tabs" className="flex min-w-0 flex-1 overflow-x-auto">
        {tabs.map((t) => {
          const Icon = KIND_ICON[t.kind]
          const selected = t.id === active
          const title = tabTitle(t)
          return (
            <div
              key={t.id}
              data-tab-id={t.id}
              data-tab-kind={t.kind}
              className={cn(
                'flex shrink-0 items-center border-r border-border',
                selected ? 'bg-background text-foreground' : 'text-muted-foreground',
              )}
            >
              <button
                type="button"
                role="tab"
                id={`tab-${t.id}`}
                aria-selected={selected}
                aria-controls={`tabpanel-${paneId}`}
                title={`${KIND_LABEL[t.kind]}: ${title}`}
                onClick={() => focusTab(t.id)}
                className={cn(
                  'flex h-7 max-w-44 items-center gap-1 px-2 text-xs outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring',
                  selected && 'border-b-2 border-primary',
                )}
              >
                <Icon aria-hidden className="size-3.5 shrink-0" />
                <span className="sr-only">{KIND_LABEL[t.kind]} </span>
                <span className="truncate">{title}</span>
              </button>
              <button
                type="button"
                aria-label={`Close ${KIND_LABEL[t.kind].toLowerCase()} ${title}`}
                onClick={() => closeTab(t.id)}
                className="flex h-7 w-6 items-center justify-center hover:text-destructive focus-visible:ring-1 focus-visible:ring-ring outline-none"
              >
                <XIcon aria-hidden className="size-3" />
              </button>
            </div>
          )
        })}
      </div>
      {current && (
        <div className="flex shrink-0 items-center border-l border-border px-0.5">
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Move tab left"
            disabled={at <= 0}
            onClick={() => reorderTab(current.id, -1)}
          >
            <CaretLeftIcon aria-hidden />
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Move tab right"
            disabled={at < 0 || at >= tabs.length - 1}
            onClick={() => reorderTab(current.id, 1)}
          >
            <CaretRightIcon aria-hidden />
          </Button>
          {index !== null && (
            <>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="Move tab to the pane on the left"
                disabled={index === 0}
                onClick={() => moveTab(current.id, index - 1)}
              >
                <ArrowLeftIcon aria-hidden />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="Move tab to the pane on the right"
                disabled={index >= paneCount - 1}
                onClick={() => moveTab(current.id, index + 1)}
              >
                <ArrowRightIcon aria-hidden />
              </Button>
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label="Split: move tab to a new pane"
                disabled={paneCount >= MAX_PANES || tabs.length < 2}
                onClick={() => moveTab(current.id, 'new')}
              >
                <SquareSplitHorizontalIcon aria-hidden />
              </Button>
            </>
          )}
        </div>
      )}
    </div>
  )
}
