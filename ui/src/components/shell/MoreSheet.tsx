/**
 * 032-ui-shell-a11y: phone "More" sheet — every tool pane plus the section navigator and the
 * fleet dashboard, so the bottom bar needs only four primary tabs.
 */
import { ListBulletsIcon, SquaresFourIcon } from '@phosphor-icons/react'

import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { useStore } from '@/store'

import { ALL_TOOL_TABS } from './stack-tabs'
import { useOpenTool } from './useOpenTool'

const ROW =
  'flex min-h-[var(--touch-target)] w-full items-center gap-3 px-4 text-left text-sm outline-none hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset'

export function MoreSheet() {
  const open = useStore((s) => s.shell.moreOpen)
  const setShell = useStore((s) => s.setShell)
  const setNavOpen = useStore((s) => s.setNavOpen)
  const openTool = useOpenTool()
  const close = () => setShell({ moreOpen: false })

  return (
    <Sheet open={open} onOpenChange={(o) => setShell({ moreOpen: o })}>
      <SheetContent
        side="bottom"
        data-testid="more-sheet"
        className="max-h-[80dvh] gap-0 overflow-y-auto p-0"
      >
        <SheetHeader className="border-b border-border">
          <SheetTitle className="font-display tracking-wider uppercase">More</SheetTitle>
          <SheetDescription className="sr-only">Sections and tools</SheetDescription>
        </SheetHeader>
        <button
          type="button"
          className={ROW}
          onClick={() => {
            close()
            setNavOpen(true)
          }}
        >
          <ListBulletsIcon aria-hidden="true" className="size-4" /> Sections
        </button>
        <button
          type="button"
          className={ROW}
          onClick={() => {
            close()
            openTool('fleet')
          }}
        >
          <SquaresFourIcon aria-hidden="true" className="size-4" /> Fleet
        </button>
        {ALL_TOOL_TABS.map((t) => (
          <button
            key={t.label}
            type="button"
            className={ROW}
            onClick={() => {
              close()
              if (t.tool) openTool(t.tool)
            }}
          >
            <t.icon aria-hidden="true" className="size-4" /> {t.label}
          </button>
        ))}
      </SheetContent>
    </Sheet>
  )
}
