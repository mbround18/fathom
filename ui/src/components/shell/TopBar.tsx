import { ListIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import { ContextPicker } from '@/components/ContextPicker'
import { NamespacePicker } from '@/components/NamespacePicker'
import type { WidthClass } from '@/layout/width-class'
import { useStore } from '@/store'
import { BrandMark } from '@/theme/BrandMark'

export function TopBar({ widthClass }: { widthClass: WidthClass }) {
  const narrow = widthClass === 'phone' || widthClass === 'tablet'
  const setNavOpen = useStore((s) => s.setNavOpen)

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
      <div className="ml-auto flex min-w-0 items-center gap-2">
        <ContextPicker className="max-w-[40vw] sm:max-w-64" />
        <NamespacePicker className="max-w-[36vw] sm:max-w-56" />
      </div>
    </header>
  )
}
