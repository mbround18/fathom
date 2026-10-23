import * as React from 'react'

import { cn } from '@companion/theme'

/**
 * Horizontally scrolling strip (tab bars) that fades its clipped edges so a cut-off tab reads as
 * "more this way" rather than as a rendering bug. Keeps the active tab (`[aria-selected=true]`,
 * `[aria-pressed=true]`, `[data-active]`) in view.
 */
export function ScrollFade({ className, children, ...props }: React.ComponentProps<'div'>) {
  const ref = React.useRef<HTMLDivElement>(null)
  const [edges, setEdges] = React.useState({ start: false, end: false })

  const measure = React.useCallback(() => {
    const el = ref.current
    if (!el) return
    const start = el.scrollLeft > 1
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 1
    setEdges((prev) => (prev.start === start && prev.end === end ? prev : { start, end }))
  }, [])

  React.useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    if (el.firstElementChild) ro.observe(el.firstElementChild)
    const mo = new MutationObserver(() => {
      measure()
      el.querySelector<HTMLElement>(
        '[aria-selected="true"],[aria-pressed="true"],[data-active]',
      )?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
    })
    mo.observe(el, { subtree: true, attributes: true, attributeFilter: ['aria-selected', 'aria-pressed', 'data-active'] })
    return () => {
      ro.disconnect()
      mo.disconnect()
    }
  }, [measure])

  return (
    <div
      ref={ref}
      onScroll={measure}
      data-slot="scroll-fade"
      data-fade-start={edges.start || undefined}
      data-fade-end={edges.end || undefined}
      className={cn('no-scrollbar min-w-0 overflow-x-auto overflow-y-hidden', className)}
      {...props}
    >
      {children}
    </div>
  )
}
