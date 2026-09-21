import { cn } from '@/lib/utils'

import { BRAND_GLYPH } from './fonts'

interface BrandMarkProps {
  className?: string
  /** Hide the FATHOM wordmark (e.g. on the narrowest phones). */
  compact?: boolean
}

/**
 * 尋 (hiro): the traditional Japanese unit for a fathom, brushed, then FATHOM in the angular
 * display face. Glows on dark; glitches on hover when html[data-glitch="on"].
 */
export function BrandMark({ className, compact = false }: BrandMarkProps) {
  return (
    <span
      role="img"
      aria-label="Fathom"
      className={cn('glitch inline-flex items-center gap-2 select-none', className)}
    >
      <span
        aria-hidden="true"
        className="glow-text font-brand text-2xl leading-none text-accent-neon"
      >
        {BRAND_GLYPH}
      </span>
      {!compact && (
        <span
          aria-hidden="true"
          className="font-display text-sm leading-none font-bold tracking-[0.32em] text-foreground"
        >
          FATHOM
        </span>
      )}
    </span>
  )
}
