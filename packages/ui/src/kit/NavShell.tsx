/**
 * The signed-in frame (spec 065 R7): a dense top bar, the grouped section navigation (a sidebar on
 * wide screens, a scrolling strip on narrow ones) and the routed page. Mobile first.
 *
 * It only draws. The app decides which groups and items a person sees, counts the badges, and
 * fills the header's end (account menu, connection state) and the banner under it.
 */
import type { Icon } from '@phosphor-icons/react'
import { useEffect, useRef, type ReactNode } from 'react'

import { LABEL } from './form'
import { Link as KitLink } from './Link'
import { useKit } from './provider'

export interface NavItem {
  path: string
  label: string
  icon: Icon
  /**
   * A count beside the label, hidden at 0. Neutral, never the accent: it is not where you are.
   * `label` is read out on the link instead, e.g. "3 runs waiting for your approval".
   */
  badge?: { count: number; label: string }
  /** A dot after the label (e.g. an update is available); `label` is read out, e.g. "update available". */
  dot?: { label: string }
}

export interface NavGroup {
  /** Also the group's `data-nav-group`. */
  id: string
  /** The group's heading, in sentence case. */
  label: string
  items: readonly NavItem[]
}

const NAV_LINK =
  'relative inline-flex h-8 shrink-0 items-center gap-2 border-b border-transparent px-3 text-sm text-muted-foreground ' +
  'hover:text-foreground focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none ' +
  'aria-[current=page]:border-accent-neon aria-[current=page]:text-primary md:border-b-0 md:border-l md:aria-[current=page]:bg-accent-neon/5'

function NavLink({
  item: { path: to, label, icon: Icon, badge, dot },
  path,
  className = '',
}: {
  item: NavItem
  path: string
  className?: string
}) {
  const { Link = KitLink } = useKit()
  const current = path === to || path.startsWith(`${to}/`)
  const dotRef = useRef<HTMLSpanElement>(null)
  const hasDot = dot !== undefined
  // On a phone the strip scrolls and the pinned item is last, so bring the dot into view when it
  // appears, then the current page: when both don't fit, the page the user is on wins.
  useEffect(() => {
    if (!hasDot) return
    const dot = dotRef.current
    dot?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    dot?.closest('nav')?.querySelector('[aria-current="page"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [hasDot])
  return (
    <Link to={to} aria-current={current ? 'page' : undefined} className={`${NAV_LINK} ${className}`}>
      <Icon aria-hidden className="size-4 shrink-0" />
      {label}
      {/* The link is `relative` so the sr-only text stays inside the phone's scrolling strip
          instead of widening the page from wherever the strip scrolled it to. */}
      {badge !== undefined && badge.count > 0 && (
        <>
          <span
            aria-hidden
            data-nav-badge={to.slice(1)}
            className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center bg-muted px-1 text-[0.6875rem] leading-none font-medium text-foreground tabular-nums"
          >
            {badge.count}
          </span>
          <span className="sr-only">
            {' '}
            ({badge.label})
          </span>
        </>
      )}
      {dot && (
        <>
          <span ref={dotRef} aria-hidden data-update-dot className="size-1.5 shrink-0 rounded-full bg-accent-neon shadow-[var(--glow)]" />
          <span className="sr-only"> ({dot.label})</span>
        </>
      )}
    </Link>
  )
}

/**
 * One group of items. On desktop its heading is a muted label, never the accent, with room above
 * every group after the first. On a phone the heading is only read out and the group runs on in the
 * strip, set off from the one before by a 1px rule.
 */
function NavGroupLinks({ group, first, path }: { group: NavGroup; first: boolean; path: string }) {
  const headingId = `nav-group-${group.id}`
  return (
    <div
      role="group"
      aria-labelledby={headingId}
      data-nav-group={group.id}
      className={`relative flex shrink-0 gap-0.5 md:flex-col ${first ? '' : 'max-md:border-l max-md:border-border max-md:pl-0.5 md:mt-3'}`}
    >
      {/* The transparent border lines the heading up with the links' text. The group is `relative` so
          the sr-only heading stays inside the phone's scrolling strip, like the dot's text. */}
      <span id={headingId} className={`${LABEL} sr-only md:not-sr-only md:border-l md:border-transparent md:px-3 md:pb-1`}>
        {group.label}
      </span>
      {group.items.map((item) => (
        <NavLink key={item.path} item={item} path={path} />
      ))}
    </div>
  )
}

export function NavShell({
  brand,
  groups,
  path,
  pinned,
  headerEnd,
  banner,
  overlay,
  children,
}: {
  /** The header's start, usually the brand mark linking home. */
  brand: ReactNode
  groups: readonly NavGroup[]
  /** The current path, for `aria-current`. */
  path: string
  /** Pinned to the bottom of the column on desktop; last in the strip on a phone (e.g. Settings). */
  pinned?: NavItem | undefined
  /** The header's end: connection state, account menu. */
  headerEnd?: ReactNode
  /** A full-width notice under the header. */
  banner?: ReactNode
  /** Rendered after the frame, e.g. lazily loaded dialogs. */
  overlay?: ReactNode
  /** The routed page. */
  children: ReactNode
}) {
  return (
    <div className="flex h-full flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-2">
        {brand}
        <span className="flex-1" />
        {headerEnd}
      </header>
      {banner}
      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <nav
          aria-label="Sections"
          className="flex shrink-0 gap-0.5 overflow-x-auto border-b border-border px-1 py-1 md:w-48 md:flex-col md:overflow-x-visible md:border-r md:border-b-0 md:py-2"
        >
          {groups.map((group, i) => (
            <NavGroupLinks key={group.id} group={group} first={i === 0} path={path} />
          ))}
          {pinned && <NavLink item={pinned} path={path} className="md:mt-auto md:border-t md:border-border" />}
        </nav>
        <main className="min-h-0 flex-1 overflow-auto">{children}</main>
      </div>
      {overlay}
    </div>
  )
}
