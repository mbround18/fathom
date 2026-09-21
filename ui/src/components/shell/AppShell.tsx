import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { NavTree } from '@/components/NavTree'
import { Toaster } from '@/components/ui/sonner'
import { useWidthClass } from '@/layout/use-width-class'
import { useStore } from '@/store'

import { PhoneStack } from './PhoneStack'
import { SlotGrid } from './SlotGrid'
import { StatusBar } from './StatusBar'
import { TabletTabs } from './TabletTabs'
import { TopBar } from './TopBar'

/**
 * Top bar, a body chosen by width class, status bar (contracts/layout.md). Test hooks:
 * `data-width-class` here, and `data-slot="pane"` / `data-pane-kind` on every pane host.
 */
export function AppShell() {
  const { widthClass } = useWidthClass()
  const navOpen = useStore((s) => s.layout.navOpen)
  const setNavOpen = useStore((s) => s.setNavOpen)
  const narrow = widthClass === 'phone' || widthClass === 'tablet'

  return (
    <div
      data-width-class={widthClass}
      className="grain flex h-dvh w-full flex-col overflow-hidden bg-background text-foreground"
    >
      <TopBar widthClass={widthClass} />
      <main className="flex min-h-0 flex-1 flex-col">
        {widthClass === 'phone' ? (
          <PhoneStack />
        ) : widthClass === 'tablet' ? (
          <TabletTabs />
        ) : (
          <SlotGrid />
        )}
      </main>
      <StatusBar />
      {narrow && (
        <Sheet open={navOpen} onOpenChange={setNavOpen}>
          <SheetContent side="left" className="w-72 overflow-y-auto p-0 text-sm">
            <SheetHeader className="border-b border-border">
              <SheetTitle className="font-display tracking-wider uppercase">Navigator</SheetTitle>
              <SheetDescription className="sr-only">Choose a resource section</SheetDescription>
            </SheetHeader>
            <NavTree />
          </SheetContent>
        </Sheet>
      )}
      <Toaster position={narrow ? 'top-center' : 'bottom-right'} />
    </div>
  )
}
