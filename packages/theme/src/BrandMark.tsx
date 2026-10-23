import { cn } from './cn'

interface BrandMarkProps {
  /** The app's name: the accessible label, and the wordmark (shown upper-cased). */
  name: string
  /** The brand glyph, rendered in --font-brand. */
  glyph: string
  className?: string
  /** Hide the wordmark (e.g. on the narrowest phones). */
  compact?: boolean
}

/**
 * The brand glyph, brushed, then the app's name in the angular display face. Glows on dark;
 * glitches on hover when html[data-glitch="on"].
 */
export function BrandMark({ name, glyph, className, compact = false }: BrandMarkProps) {
  return (
    <span
      role="img"
      aria-label={name}
      className={cn('glitch inline-flex items-center gap-2 select-none', className)}
    >
      <span
        aria-hidden="true"
        className="glow-text font-brand text-2xl leading-none text-accent-neon"
      >
        {glyph}
      </span>
      {!compact && (
        <span
          aria-hidden="true"
          className="font-display text-sm leading-none font-bold tracking-[0.32em] text-foreground"
        >
          {name.toUpperCase()}
        </span>
      )}
    </span>
  )
}
