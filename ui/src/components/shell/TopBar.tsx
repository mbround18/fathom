import { ListIcon, MagnifyingGlassIcon, SquaresFourIcon } from '@phosphor-icons/react'

import { FleetDashboardView } from '@/components/fleet/FleetDashboardView'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ContextPicker } from '@/components/ContextPicker'
import { NamespacePicker } from '@/components/NamespacePicker'
import type { WidthClass } from '@/layout/width-class'
import { useStore } from '@/store'
import { BrandMark } from '@/theme/BrandMark'

import { AskAiButton, useLoadAssistantMode } from './AskAi'
import { OverflowMenu } from './OverflowMenu'
import { ScopeChip } from './ScopeSheet'
import { ToolsMenu } from './ToolsSheet'

export function TopBar({ widthClass }: { widthClass: WidthClass }) {
  const phone = widthClass === 'phone'
  const narrow = phone || widthClass === 'tablet'
  const setNavOpen = useStore((s) => s.setNavOpen)
  const setShell = useStore((s) => s.setShell)
  const fleetOpen = useStore((s) => s.shell.fleetOpen)
  const setFleetOpen = (open: boolean) => setShell({ fleetOpen: open })
  useLoadAssistantMode()

  return (
    <header className="neon-chrome flex h-11 shrink-0 items-center gap-2 border-b border-border bg-card px-2 sm:px-3">
      {narrow && (
        <Button
          variant="ghost"
          size="icon"
          aria-label="Open sections"
          onClick={() => setNavOpen(true)}
          className="size-[var(--touch-target)] shrink-0"
        >
          <ListIcon />
        </Button>
      )}
      <BrandMark compact={phone} className="shrink-0" />
      {phone ? (
        <ScopeChip />
      ) : (
        <div className="flex min-w-0 items-center gap-2">
          <ContextPicker className="max-w-[40vw] sm:max-w-64" />
          <NamespacePicker className="max-w-[36vw] sm:max-w-56" />
        </div>
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1" role="toolbar" aria-label="Actions">
        <Button
          variant="outline"
          data-testid="palette-open"
          aria-label="Search"
          aria-keyshortcuts="Control+K Meta+K"
          onClick={() => setShell({ paletteOpen: true })}
          className={
            phone
              ? 'size-[var(--touch-target)] px-0'
              : 'h-[var(--touch-target)] gap-2 border-border/80 px-2 text-muted-foreground'
          }
        >
          <MagnifyingGlassIcon aria-hidden="true" data-icon="inline-start" />
          {!phone && (
            <>
              <span>Search</span>
              <kbd className="border border-border px-1 font-mono text-xs font-normal normal-case">
                Ctrl K
              </kbd>
            </>
          )}
        </Button>
        {!phone && <AskAiButton />}
        {!phone && (
          <Button
            variant="ghost"
            size="icon"
            aria-label="Fleet dashboard"
            data-testid="fleet-open"
            onClick={() => setFleetOpen(true)}
            className="size-[var(--touch-target)] shrink-0"
          >
            <SquaresFourIcon aria-hidden="true" />
          </Button>
        )}
        {widthClass === 'laptop' && <ToolsMenu />}
        <OverflowMenu folded={phone} onOpenFleet={() => setFleetOpen(true)} />
      </div>
      <Dialog open={fleetOpen} onOpenChange={setFleetOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-4xl">
          <DialogHeader>
            <DialogTitle>Fleet</DialogTitle>
            <DialogDescription>Health of every kubeconfig context.</DialogDescription>
          </DialogHeader>
          <FleetDashboardView />
        </DialogContent>
      </Dialog>
    </header>
  )
}
