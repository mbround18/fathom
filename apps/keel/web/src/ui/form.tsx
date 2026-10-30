/**
 * Form pieces every page shares (spec 060 T053): one input and select style, one field label, and
 * a labelled field. Native selects are kept on purpose: dense, keyboard and touch friendly, and
 * what Playwright's `selectOption` drives.
 */
import type { ComponentProps, ReactNode } from 'react'

import type { GuideRef } from '@/help/guides'

import { GuideLink } from './GuideLink'

/** The field look without a size: border, focus ring, disabled and invalid states. */
const FIELD_LOOK =
  'min-w-0 border border-input bg-background text-sm outline-none transition-colors ' +
  'focus-visible:border-accent-neon focus-visible:ring-1 focus-visible:ring-accent-neon/60 ' +
  'disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-status-err'

/** A native input or select, styled like `@companion/ui`'s Input, with a visible focus ring. */
export const FIELD = `h-8 px-2 ${FIELD_LOOK}`

/** The same, a step shorter, for a control inside a dense list row. */
export const FIELD_DENSE = `h-7 px-1 ${FIELD_LOOK}`

/** A field's label: sentence case, a step quieter than the value it names. */
export const LABEL = 'text-xs font-medium text-muted-foreground'

/** A short status word (a run's or a host's state): its tone comes from the caller. */
export const STATUS = 'inline-block text-xs font-medium whitespace-nowrap first-letter:uppercase'

/** How wide a form grows: fields are for names, URLs and choices, never a full screen of line. */
export const FORM_WIDTH = 'max-w-2xl'

/** A text button or link that opens a record: neon on hover, a visible focus ring. */
export const LINK =
  'min-w-0 text-left text-primary underline-offset-4 hover:underline focus-visible:underline ' +
  'focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none'

/** A page's title (h1). The type scale: H1, then H2 for a section, H3 for a group inside one. */
export const H1 = 'font-display text-2xl font-semibold'

/** A section heading inside a page (h2). */
export const H2 = 'font-display text-base font-semibold'

/** A group's heading inside a section (h3): members, grants, the parts of a card. */
export const H3 = 'text-sm font-semibold'

/**
 * A section's one-line intro under its heading, then "Learn more" (spec 063 R8, FR-023). Anything
 * longer than a line belongs in the guide it links to, not on the page.
 */
export function SectionIntro({ guide, children }: { guide: GuideRef; children: ReactNode }) {
  return (
    <div data-section-intro className="flex max-w-prose min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
      <p className="min-w-0 text-xs text-muted-foreground">{children}</p>
      <GuideLink to={guide} />
    </div>
  )
}

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
      className={`flex min-w-0 flex-col gap-3 border border-border bg-muted/20 p-4 ${FORM_WIDTH} ${className}`}
    >
      <h2 className={H2}>{label}</h2>
      {children}
    </form>
  )
}
