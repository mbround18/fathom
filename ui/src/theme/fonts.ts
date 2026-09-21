/**
 * Self-hosted fonts (contracts/theme.md rule 7): bundled from @fontsource, never a CDN, so the
 * app works offline. The CSS families are wired to Tailwind in tokens.css (`@theme inline`):
 *
 *   --font-mono / --font-sans  JetBrains Mono 400/600   body, tables, identifiers, logs, YAML
 *   --font-display             Chakra Petch 600/700     headings, pane titles, chrome, buttons
 *   --font-brand               Yuji Boku 400            the 尋 mark only
 *
 * Import this module once, from main.tsx.
 */
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/jetbrains-mono/600.css'
import '@fontsource/chakra-petch/600.css'
import '@fontsource/chakra-petch/700.css'

// Yuji Boku ships ~120 unicode-range chunks (~9 MB) for full Japanese coverage. The brand only
// ever renders 尋 (U+5C0B), so register just the fontsource chunk that contains it. If a
// fontsource update renumbers its chunks this import fails the build, rather than silently
// falling back to the serif stack.
import brandGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-84-400-normal.woff2?url'

export const BRAND_GLYPH = '尋'

export const FONT_STACKS = {
  mono: "'JetBrains Mono', ui-monospace, monospace",
  display: "'Chakra Petch', 'JetBrains Mono', ui-monospace, monospace",
  brand: "'Yuji Boku', 'Noto Serif JP', serif",
} as const

function registerBrandFont(): void {
  if (typeof FontFace === 'undefined' || typeof document === 'undefined') return
  const face = new FontFace('Yuji Boku', `url(${brandGlyphUrl}) format('woff2')`, {
    weight: '400',
    style: 'normal',
    display: 'swap',
    unicodeRange: 'U+5C0B',
  })
  document.fonts.add(face)
  // Load eagerly: the mark is in the top bar on first paint.
  void face.load().catch(() => {
    /* Falls back to the serif stack in --font-brand. */
  })
}

registerBrandFont()
