/**
 * 032-ui-shell-a11y: tool panes as an overlay sheet, for widths with no dock and no tab strip
 * (laptop). A "Tools" menu in the top bar picks the tool; the sheet hosts its regular `PaneHost`
 * (state lives in an off-layout pane, so it survives close/reopen).
 */
import { useFocusReturn } from '@/lib/use-focus-return'
import { WrenchIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { cn } from '@companion/theme'
import { useStore } from '@/store'

import { PaneHost } from './PaneHost'
import { ALL_TOOL_TABS, useVisibleToolTabs } from './stack-tabs'

export function ToolsMenu() {
  const setShell = useStore((s) => s.setShell)
  // Velero is listed only once detected (043 FR-036); every other tool stays (Tools is kept).
  const toolTabs = useVisibleToolTabs(ALL_TOOL_TABS)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          data-testid="tools-menu"
          className="h-[var(--touch-target)] shrink-0 gap-1.5 px-2"
        >
          <WrenchIcon aria-hidden="true" data-icon="inline-start" />
          Tools
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {toolTabs.map((t) => (
          <DropdownMenuItem
            key={t.label}
            onSelect={() => t.tool && setShell({ toolsSheet: t.tool })}
          >
            <t.icon aria-hidden="true" />
            {t.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function ToolsSheet() {
  const tool = useStore((s) => s.shell.toolsSheet)
  const setShell = useStore((s) => s.setShell)
  const ensureToolPane = useStore((s) => s.ensureToolPane)
  const paneId = tool ? ensureToolPane(tool) : undefined
  const returnFocus = useFocusReturn(tool !== undefined)
  const toolTabs = useVisibleToolTabs(ALL_TOOL_TABS)

  return (
    <Sheet
      open={tool !== undefined}
      onOpenChange={(o) => !o && setShell({ toolsSheet: undefined })}
    >
      <SheetContent
        side="right"
        onCloseAutoFocus={returnFocus}
        data-testid="tools-sheet"
        // The base sheet pins `data-[side=right]:w-3/4` and `data-[side=right]:sm:max-w-sm`
        // (~24rem); a plain `sm:max-w-3xl` loses to those variants, which squeezed every tool
        // (the topology canvas most of all) into a ~310px strip. Override the same variants.
        className="w-full max-w-full gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-3xl data-[side=right]:xl:max-w-5xl"
      >
        <SheetHeader className="border-b border-border pr-12">
          <SheetTitle className="font-display tracking-wider uppercase">Tools</SheetTitle>
          <SheetDescription className="sr-only">Open a tool pane</SheetDescription>
          <div role="group" aria-label="Tools" className="-mx-1 flex flex-wrap gap-1">
            {toolTabs.map((t) => (
              <button
                key={t.label}
                type="button"
                aria-pressed={t.tool === tool}
                onClick={() => t.tool && setShell({ toolsSheet: t.tool })}
                className={cn(
                  'min-h-6 px-2 font-display text-xs font-semibold tracking-wider uppercase outline-none focus-visible:ring-1 focus-visible:ring-ring',
                  t.tool === tool
                    ? 'bg-primary text-primary-foreground'
                    : 'text-muted-foreground hover:text-foreground',
                )}
              >
                {t.label}
              </button>
            ))}
          </div>
        </SheetHeader>
        <div className="min-h-0 flex-1">{paneId && <PaneHost key={paneId} paneId={paneId} />}</div>
      </SheetContent>
    </Sheet>
  )
}
