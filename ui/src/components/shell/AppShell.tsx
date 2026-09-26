import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { ActionDialogHost } from '@/components/ActionDialogHost'
import { NavTree } from '@/components/NavTree'
import { KubeconfigExportDialogHost } from '@/features/manifest-export/KubeconfigExportDialog'
import { ManifestExportDialogHost } from '@/features/manifest-export/ManifestExportDialog'
import { Toaster } from '@/components/ui/sonner'
import { useWidthClass } from '@/layout/use-width-class'
import { useStore } from '@/store'

import { AskAiSheet } from './AskAi'
import { MoreSheet } from './MoreSheet'
import { ScopeSheet } from './ScopeSheet'
import { ToolsSheet } from './ToolsSheet'
import { ClusterVersionBadge } from './ClusterVersionBadge'
import { CommandPaletteHost } from './CommandPaletteHost'
import { ShortcutsHost } from './ShortcutsHost'
import { FederationContextSwitcher } from './FederationContextSwitcher'
import { PhoneStack } from './PhoneStack'
import { SlotGrid } from './SlotGrid'
import { StatusBar } from './StatusBar'
import { TabletTabs } from './TabletTabs'
import { TopBar } from './TopBar'

/** Top bar height (`h-11`) plus a small gap. */
const TOAST_TOP = 52

/**
 * Top bar, a body chosen by width class, status bar (contracts/layout.md). Test hooks:
 * `data-width-class` here, and `data-slot="pane"` / `data-pane-kind` on every pane host.
 */
export function AppShell() {
  const { widthClass } = useWidthClass()
  const navOpen = useStore((s) => s.layout.navOpen)
  const setNavOpen = useStore((s) => s.setNavOpen)
  const isHosted = useStore((s) => s.hosted.mode === 'hosted')
  const narrow = widthClass === 'phone' || widthClass === 'tablet'

  return (
    <div
      data-width-class={widthClass}
      className="grain flex h-dvh w-full flex-col overflow-hidden bg-background text-foreground"
    >
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-1 focus:left-1 focus:z-[100] focus:border focus:border-accent-neon focus:bg-background focus:px-3 focus:py-2 focus:text-xs focus:text-accent-neon"
      >
        Skip to main content
      </a>
      <h1 className="sr-only">Fathom</h1>
      <TopBar widthClass={widthClass} />
      <div className="flex w-full shrink-0 items-center">
        <div className="min-w-0 flex-1">{!isHosted && <FederationContextSwitcher />}</div>
        <ClusterVersionBadge />
      </div>
      <main id="main-content" tabIndex={-1} className="flex min-h-0 flex-1 flex-col outline-none">
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
              <SheetTitle className="font-display tracking-wider uppercase">Sections</SheetTitle>
              <SheetDescription className="sr-only">Choose a resource section</SheetDescription>
            </SheetHeader>
            <NavTree />
          </SheetContent>
        </Sheet>
      )}
      <ActionDialogHost />
      <ManifestExportDialogHost />
      <KubeconfigExportDialogHost />
      <CommandPaletteHost />
      <ShortcutsHost />
      <AskAiSheet />
      {widthClass === 'phone' && <ScopeSheet />}
      {widthClass === 'phone' && <MoreSheet />}
      {!narrow && <ToolsSheet />}
      {/* Narrow: toasts drop below the 44px top bar; error toasts stay until dismissed and must
          never cover the context/namespace pickers. */}
      <Toaster
        position={narrow ? 'top-center' : 'bottom-right'}
        {...(narrow && { offset: { top: TOAST_TOP }, mobileOffset: { top: TOAST_TOP } })}
      />
    </div>
  )
}
