import { cn } from '@/lib/utils'

interface CircuitTraceProps {
  className?: string
}

/**
 * Circuit-trace line art for empty, error, and first-run states (contracts/theme.md § Effects).
 * Decorative only: aria-hidden, structure in --border, live traces in --accent-neon.
 */
export function CircuitTrace({ className }: CircuitTraceProps) {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      viewBox="0 0 240 96"
      fill="none"
      strokeWidth="1.5"
      strokeLinecap="square"
      className={cn('h-auto w-48 max-w-full', className)}
    >
      <g stroke="var(--border)">
        <path d="M0 20 H52 L64 32 H108" />
        <path d="M0 76 H40 L52 64 H92 L104 52 H132" />
        <path d="M240 18 H196 L184 30 H152" />
        <path d="M240 78 H206 L194 66 H164 L152 54 H132" />
        <path d="M120 0 V14 L132 26 V40" />
        <path d="M120 96 V80" />
      </g>
      <g stroke="var(--accent-neon)">
        <path d="M108 32 H124 L132 40 V52" />
        <path d="M152 30 H142 L132 40" />
        <rect x="112" y="80" width="16" height="10" />
      </g>
      <g fill="var(--accent-neon)">
        <rect x="129" y="49" width="6" height="6" />
        <rect x="105" y="29" width="6" height="6" />
        <rect x="149" y="27" width="6" height="6" />
      </g>
      <g fill="var(--background)" stroke="var(--border)">
        <circle cx="52" cy="20" r="3" />
        <circle cx="92" cy="64" r="3" />
        <circle cx="196" cy="18" r="3" />
        <circle cx="164" cy="66" r="3" />
        <circle cx="120" cy="14" r="3" />
      </g>
    </svg>
  )
}
