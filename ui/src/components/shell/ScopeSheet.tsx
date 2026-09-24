/**
 * 032-ui-shell-a11y: phone-width replacement for the two top-bar pickers — one chip showing the
 * active context and namespace, opening a sheet that hosts the same pickers.
 */
import { CaretDownIcon } from '@phosphor-icons/react'

import { ContextPicker } from '@/components/ContextPicker'
import { NamespacePicker } from '@/components/NamespacePicker'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { useStore } from '@/store'

export function ScopeChip() {
  const context = useStore((s) => s.session.activeContext)
  const namespace = useStore((s) => s.session.activeNamespace)
  const setShell = useStore((s) => s.setShell)
  return (
    <button
      type="button"
      data-testid="scope-chip"
      aria-label="Context and namespace"
      onClick={() => setShell({ scopeOpen: true })}
      className="flex h-[var(--touch-target)] min-w-0 flex-1 items-center gap-1.5 border border-border/80 bg-background/40 px-2 font-mono text-xs outline-none hover:border-accent-neon/60 focus-visible:ring-1 focus-visible:ring-ring"
    >
      <span className="min-w-0 truncate text-accent-neon">{context ?? 'no context'}</span>
      <span aria-hidden="true" className="text-muted-foreground">
        /
      </span>
      <span className="min-w-0 truncate">{namespace ?? 'all'}</span>
      <CaretDownIcon aria-hidden="true" className="ml-auto size-3.5 shrink-0 text-muted-foreground" />
    </button>
  )
}

export function ScopeSheet() {
  const open = useStore((s) => s.shell.scopeOpen)
  const setShell = useStore((s) => s.setShell)
  return (
    <Sheet open={open} onOpenChange={(o) => setShell({ scopeOpen: o })}>
      <SheetContent side="top" className="gap-0 p-0">
        <SheetHeader className="border-b border-border">
          <SheetTitle className="font-display tracking-wider uppercase">Scope</SheetTitle>
          <SheetDescription className="sr-only">Choose the context and namespace</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-3 p-4">
          <ContextPicker className="w-full max-w-full" />
          <NamespacePicker className="w-full max-w-full" />
        </div>
      </SheetContent>
    </Sheet>
  )
}
