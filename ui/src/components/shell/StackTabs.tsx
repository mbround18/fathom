import { DotsThreeIcon, type Icon } from '@phosphor-icons/react'

import { ScrollFade } from '@/components/ui/scroll-fade'
import { cn } from '@companion/theme'
import { useStore } from '@/store'

import { useActiveTab, type StackTab } from './stack-tabs'

interface StackTabsProps {
  tabs: readonly StackTab[]
  /** Adds a "More" tab that opens the tools sheet (phone). */
  withMore?: boolean
  /** Scroll the strip with edge fades instead of squeezing every tab (tablet). */
  scroll?: boolean
  className?: string
}

/** Tab strip that swaps the stack root (phone bottom bar, tablet top strip). */
export function StackTabs({ tabs, withMore = false, scroll = false, className }: StackTabsProps) {
  const openRoot = useStore((s) => s.openRoot)
  const setShell = useStore((s) => s.setShell)
  const active = useActiveTab(tabs)
  // On phone the active root may be a tool that only "More" lists; light up More then.
  const moreActive = withMore && active === undefined

  const item = (
    key: string,
    label: string,
    IconCmp: Icon,
    selected: boolean,
    onClick: () => void,
  ) => (
    <button
      key={key}
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        'flex min-h-[var(--touch-target)] min-w-0 flex-col items-center justify-center gap-0.5 px-1 font-display text-xs font-semibold tracking-wide outline-none',
        scroll ? 'flex-none px-3' : 'flex-1',
        'hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
        selected
          ? 'bg-primary text-primary-foreground hover:text-primary-foreground'
          : 'text-muted-foreground',
      )}
    >
      <IconCmp aria-hidden="true" className="size-4" />
      <span className="max-w-full truncate">{label}</span>
    </button>
  )

  const strip = (
    <>
      {tabs.map((t) =>
        item(t.label, t.label, t.icon, t === active, () => openRoot(t.kind, t.tool)),
      )}
      {withMore &&
        item('more', 'More', DotsThreeIcon, moreActive, () => setShell({ moreOpen: true }))}
    </>
  )

  return (
    <nav aria-label="Views" className={cn('flex shrink-0 border-border bg-card', className)}>
      {scroll ? <ScrollFade className="flex w-full">{strip}</ScrollFade> : strip}
    </nav>
  )
}
