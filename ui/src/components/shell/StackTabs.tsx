import { DotsThreeIcon, type Icon } from '@phosphor-icons/react'

import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { useActiveTab, type StackTab } from './stack-tabs'

interface StackTabsProps {
  tabs: readonly StackTab[]
  /** Adds a "More" tab that opens the navigator drawer. */
  withMore?: boolean
  className?: string
}

/** Tab strip that swaps the stack root (phone bottom bar, tablet top strip). */
export function StackTabs({ tabs, withMore = false, className }: StackTabsProps) {
  const openRoot = useStore((s) => s.openRoot)
  const setNavOpen = useStore((s) => s.setNavOpen)
  const active = useActiveTab(tabs)

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
        'flex min-h-[var(--touch-target)] min-w-0 flex-1 flex-col items-center justify-center gap-0.5 px-1 font-display text-[0.65rem] font-semibold tracking-wider uppercase outline-none',
        'hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
        selected
          ? 'bg-primary text-primary-foreground hover:text-primary-foreground'
          : 'text-muted-foreground',
      )}
    >
      <IconCmp aria-hidden="true" className="size-4" />
      <span className="truncate">{label}</span>
    </button>
  )

  return (
    <nav aria-label="Views" className={cn('flex shrink-0 border-border bg-card', className)}>
      {tabs.map((t) =>
        item(t.label, t.label, t.icon, t === active, () => openRoot(t.kind, t.tool)),
      )}
      {withMore && item('more', 'More', DotsThreeIcon, false, () => setNavOpen(true))}
    </nav>
  )
}
