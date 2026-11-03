/**
 * A short list of plain values: a certificate's names, a token's scopes, a grant's paths. The
 * chips are static, so they look it: no hover, no pointer. Past `max` the rest fold into a
 * "+N more" button, and the list can fold back.
 */
import { useState } from 'react'

import { LINK } from './form'

export function ChipList({ label, items, max = 6, empty = 'None' }: { label: string; items: readonly string[]; max?: number; empty?: string }) {
  const [open, setOpen] = useState(false)
  if (items.length === 0) return <span className="text-xs text-muted-foreground">{empty}</span>
  const shown = open ? items : items.slice(0, max)
  const hidden = items.length - shown.length
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1">
      <ul aria-label={label} className="contents">
        {shown.map((item) => (
          <li key={item} className="max-w-full border border-border bg-muted/40 px-1.5 py-0.5 font-mono text-xs break-all">
            {item}
          </li>
        ))}
      </ul>
      {items.length > max && (
        <button
          type="button"
          aria-expanded={open}
          className={`${LINK} min-h-6 px-1 text-xs`}
          onClick={() => {
            setOpen((o) => !o)
          }}
        >
          {open ? 'Show fewer' : `+${String(hidden)} more`}
        </button>
      )}
    </div>
  )
}
