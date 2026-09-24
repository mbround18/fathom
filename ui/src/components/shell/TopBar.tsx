import { useState } from 'react'
import { ListIcon, SquaresFourIcon } from '@phosphor-icons/react'

import { FleetDashboardView } from '@/components/fleet/FleetDashboardView'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { ContextPicker } from '@/components/ContextPicker'
import { NamespacePicker } from '@/components/NamespacePicker'
import type { WidthClass } from '@/layout/width-class'
import { useStore } from '@/store'
import { BrandMark } from '@/theme/BrandMark'

export function TopBar({ widthClass }: { widthClass: WidthClass }) {
  const narrow = widthClass === 'phone' || widthClass === 'tablet'
  const setNavOpen = useStore((s) => s.setNavOpen)
  const [fleetOpen, setFleetOpen] = useState(false)

  return (
    <header className="neon-chrome flex h-11 shrink-0 items-center gap-2 border-b border-border bg-card px-2 sm:px-3">
      {narrow && (
        <Button
          variant="ghost"
          size="icon"
          aria-label="Open navigator"
          onClick={() => setNavOpen(true)}
          className="size-[var(--touch-target)] shrink-0"
        >
          <ListIcon />
        </Button>
      )}
      <BrandMark compact={widthClass === 'phone'} className="shrink-0" />
      <Dialog open={fleetOpen} onOpenChange={setFleetOpen}>
        <DialogTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Fleet dashboard"
            data-testid="fleet-open"
            className="ml-auto size-[var(--touch-target)] shrink-0"
          >
            <SquaresFourIcon />
          </Button>
        </DialogTrigger>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-4xl">
          <DialogHeader>
            <DialogTitle>Fleet</DialogTitle>
            <DialogDescription>Health of every kubeconfig context.</DialogDescription>
          </DialogHeader>
          <FleetDashboardView onSelected={() => setFleetOpen(false)} />
        </DialogContent>
      </Dialog>
      <div className="flex min-w-0 items-center gap-2">
        <ContextPicker className="max-w-[40vw] sm:max-w-64" />
        <NamespacePicker className="max-w-[36vw] sm:max-w-56" />
      </div>
    </header>
  )
}
