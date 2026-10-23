/**
 * Self-hosted fonts (contracts/theme.md rule 7): bundled from @fontsource, never a CDN, so the
 * app works offline. The CSS families are wired to Tailwind in tokens.css (`@theme inline`):
 *
 *   --font-mono / --font-sans  JetBrains Mono 400/600   body, tables, identifiers, logs, YAML
 *   --font-display             Chakra Petch 600/700     headings, pane titles, chrome, buttons
 *   --font-brand               Yuji Boku 400            the app's brand glyph only
 *
 * The mono and display faces come from `@companion/theme/fonts.css`, which the app imports from
 * its entry module; call `loadFonts` once, from the same place, to register the brand face.
 */

export const FONT_STACKS = {
  mono: "'JetBrains Mono', ui-monospace, monospace",
  display: "'Chakra Petch', 'JetBrains Mono', ui-monospace, monospace",
  brand: "'Yuji Boku', 'Noto Serif JP', serif",
} as const

/**
 * The one Yuji Boku chunk an app's brand glyph needs. Yuji Boku ships ~120 unicode-range chunks
 * (~9 MB) for full Japanese coverage and a brand only ever renders one glyph, so the app imports
 * just the fontsource chunk that contains it (`...woff2?url`) and names its code point. If a
 * fontsource update renumbers its chunks, that import fails the app's build rather than silently
 * falling back to the serif stack.
 */
export interface BrandFont {
  /** URL of the woff2 chunk, from a Vite `?url` import. */
  url: string
  /** CSS unicode-range of the glyph, e.g. `U+5C0B`. */
  unicodeRange: string
}

/** Register the brand face. Without a `brand`, `--font-brand` falls back to the serif stack. */
export function loadFonts(brand?: BrandFont): void {
  if (!brand || typeof FontFace === 'undefined' || typeof document === 'undefined') return
  const face = new FontFace('Yuji Boku', `url(${brand.url}) format('woff2')`, {
    weight: '400',
    style: 'normal',
    display: 'swap',
    unicodeRange: brand.unicodeRange,
  })
  document.fonts.add(face)
  // Load eagerly: the mark is in the top bar on first paint.
  void face.load().catch(() => {
    /* Falls back to the serif stack in --font-brand. */
  })
}
