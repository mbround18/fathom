import { cn } from '@/lib/utils'
import { useStore } from '@/store'

import { NAV_SECTIONS } from './labels'

/** Section navigator: used by the nav slot (laptop+) and the drawer (phone/tablet). */
export function NavList() {
  const navigate = useStore((s) => s.navigate)
  const active = useStore((s) => {
    const id = s.layout.slotPanes.find((p) => s.panes[p]?.kind === 'list')
    return id ? s.panes[id]?.route?.section : undefined
  })

  return (
    <nav aria-label="Sections" className="flex flex-col py-1">
      {NAV_SECTIONS.map(({ group, sections }) => (
        <div key={group} className="flex flex-col">
          <p className="px-2 pt-3 pb-1 font-display text-[0.65rem] font-semibold tracking-[0.2em] text-muted-foreground uppercase">
            {group}
          </p>
          {sections.map((section) => (
            <button
              key={section}
              type="button"
              aria-current={active === section ? 'page' : undefined}
              onClick={() => navigate(section)}
              className={cn(
                'flex h-[var(--row-height)] min-h-[var(--touch-target)] items-center truncate px-3 text-left outline-none',
                'hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
                active === section && 'bg-primary text-primary-foreground hover:bg-primary',
              )}
            >
              {section}
            </button>
          ))}
        </div>
      ))}
    </nav>
  )
}
