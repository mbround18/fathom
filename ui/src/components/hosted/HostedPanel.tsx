import { useEffect, useRef, type ReactNode } from 'react'

/**
 * Full-viewport, centered status card shared by the hosted gate screens. Moves focus to the
 * heading on mount so screen-reader and keyboard users land on the message; `role` is `alert`
 * for states that interrupt (session ended) and `status` otherwise.
 */
export function HostedPanel({
  title,
  role = 'status',
  children,
  actions,
}: {
  title: string
  role?: 'alert' | 'status'
  children?: ReactNode
  actions?: ReactNode
}) {
  const heading = useRef<HTMLHeadingElement>(null)
  useEffect(() => {
    heading.current?.focus()
  }, [])
  return (
    <main className="flex min-h-dvh items-center justify-center bg-background p-4">
      <section
        role={role}
        aria-labelledby="hosted-panel-title"
        className="w-full max-w-md border border-border bg-card p-5 sm:p-6"
      >
        <h1
          id="hosted-panel-title"
          ref={heading}
          tabIndex={-1}
          className="font-display text-base font-semibold tracking-wider uppercase outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {title}
        </h1>
        <div className="mt-3 space-y-2 text-sm text-muted-foreground">{children}</div>
        {actions && <div className="mt-5 flex flex-wrap gap-2">{actions}</div>}
      </section>
    </main>
  )
}
