/**
 * Form pieces every page shares (spec 060 T053): one input and select style, one field label, and
 * a labelled field. Native selects are kept on purpose: dense, keyboard and touch friendly, and
 * what Playwright's `selectOption` drives.
 */
import type { ComponentProps, ReactNode } from 'react'

/** The field look without a size: border, focus ring, disabled and invalid states. */
const FIELD_LOOK =
  'min-w-0 border border-input bg-background text-xs outline-none transition-colors ' +
  'focus-visible:border-accent-neon focus-visible:ring-1 focus-visible:ring-accent-neon/60 ' +
  'disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-status-err'

/** A native input or select, styled like `@companion/ui`'s Input, with a visible focus ring. */
export const FIELD = `h-8 px-2 ${FIELD_LOOK}`

/** The same, a step shorter, for a control inside a dense list row. */
export const FIELD_DENSE = `h-7 px-1 ${FIELD_LOOK}`

/** A field's label. */
export const LABEL = 'text-[11px] tracking-wider text-muted-foreground uppercase'

/** A text button or link that opens a record: neon on hover, a visible focus ring. */
export const LINK =
  'min-w-0 text-left text-primary underline-offset-4 hover:underline focus-visible:underline ' +
  'focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none'

/** A section heading inside a page (h2). */
export const H2 = 'font-display text-xs font-semibold tracking-wider uppercase'

export function Select({ className = '', dense = false, ...props }: ComponentProps<'select'> & { dense?: boolean }) {
  return <select {...props} className={`${dense ? FIELD_DENSE : FIELD} ${className}`} />
}

/** A labelled field: label above, control below, an optional hint under it. */
export function Field({
  label,
  htmlFor,
  hint,
  className = '',
  children,
}: {
  label: ReactNode
  htmlFor: string
  hint?: ReactNode
  className?: string
  children: ReactNode
}) {
  return (
    <div className={`flex min-w-0 flex-col gap-1 ${className}`}>
      <label htmlFor={htmlFor} className={LABEL}>
        {label}
      </label>
      {children}
      {hint !== undefined && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

/** A form in a bordered panel: its name as a heading, then the fields and the submit row. */
export function FormPanel({
  label,
  onSubmit,
  children,
  className = '',
}: {
  label: string
  onSubmit: () => void
  children: ReactNode
  className?: string
}) {
  return (
    <form
      aria-label={label}
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit()
      }}
      className={`flex min-w-0 flex-col gap-3 border border-border bg-muted/20 p-3 ${className}`}
    >
      <h2 className={H2}>{label}</h2>
      {children}
    </form>
  )
}
