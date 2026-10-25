/**
 * A container's (or stack's) containment as a small chip (051 FR-005): amber "root", red "escape",
 * nothing for ok. The reasons show on hover, on keyboard focus and on a tap (touch has no hover),
 * and are the chip's accessible description either way.
 */
import { ShieldWarningIcon } from '@phosphor-icons/react'
import { useId, useState } from 'react'

import type { ContainerSecurity, SecurityLevel } from '@/protocol/generated'

import { chipFor, reasonLines } from './security'

export function SecurityChip({
  level,
  security,
  reasons,
}: {
  level: SecurityLevel | null | undefined
  /** A container's rating, for its reasons. */
  security?: ContainerSecurity
  /** Reasons given directly (a stack's chip names its worst containers). */
  reasons?: readonly string[]
}) {
  const [open, setOpen] = useState(false)
  const tipId = useId()
  const chip = chipFor(level)
  if (!chip) return null
  const lines = reasons ?? reasonLines(security)
  const said = lines.length > 0 ? lines : [chip.level === 'escape' ? 'Can reach the host.' : 'Runs as root on the host.']
  return (
    <span className="group relative inline-flex shrink-0">
      <button
        type="button"
        data-field="security"
        data-level={chip.level}
        aria-expanded={open}
        aria-describedby={tipId}
        aria-label={`Security: ${chip.label}`}
        onClick={() => setOpen((o) => !o)}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => e.key === 'Escape' && setOpen(false)}
        className={`inline-flex min-h-5 items-center gap-0.5 border px-1 text-[0.65rem] leading-4 font-semibold tracking-wide uppercase focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none ${chip.tone}`}
      >
        <ShieldWarningIcon aria-hidden className="size-3" />
        {chip.label}
      </button>
      <span
        id={tipId}
        role="tooltip"
        data-security-reasons
        className={`absolute top-full right-0 z-30 mt-1 w-max max-w-[min(18rem,80vw)] flex-col gap-0.5 border border-border bg-popover p-1.5 text-left text-xs font-normal text-popover-foreground normal-case shadow-md group-focus-within:flex group-hover:flex ${open ? 'flex' : 'hidden'}`}
      >
        {said.map((l, i) => (
          <span key={i} className="block break-words">
            {l}
          </span>
        ))}
      </span>
    </span>
  )
}
