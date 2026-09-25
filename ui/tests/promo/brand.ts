/**
 * Promo brand assets: the same neon-noir palette and self-hosted fonts as the app
 * (src/theme/tokens.css, src/theme/fonts.ts), base64-inlined so the brochure HTML/PDF renders
 * identically with no network access. Palette values are copied by hand from tokens.css — if that
 * file's `.dark` block changes, update PALETTE to match (a Vitest check pins the copy below).
 */
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const resolve = (spec: string): string => fileURLToPath(import.meta.resolve(spec))

function fontDataUrl(spec: string): string {
  const bytes = readFileSync(resolve(spec))
  return `data:font/woff2;base64,${bytes.toString('base64')}`
}

/** `.dark` neon-noir tokens (src/theme/tokens.css), hex form for broad PDF-renderer support. */
export const PALETTE = {
  background: '#070b1a',
  card: '#0c1228',
  foreground: '#d6f6ff',
  mutedForeground: '#8a9cc9',
  accentNeon: '#3ef2ff', // cyan (default accent)
  accent2: '#ff2bd6', // magenta
  statusOk: '#39ffb0',
  statusWarn: '#ffd23f',
  statusErr: '#ff4d6d',
  border: 'rgba(196, 198, 255, 0.28)',
} as const

/**
 * `:root` "blueprint" tokens (src/theme/tokens.css) — the real light theme Fathom ships,
 * navy ink on pale paper. Used for content pages so dark neon-noir screenshots pop.
 */
export const PALETTE_LIGHT = {
  background: '#eef1fb',
  card: '#ffffff',
  foreground: '#0b1030',
  mutedForeground: '#4a5580',
  accentNeon: '#006a80', // teal ink (default accent)
  accent2: '#a0107f', // magenta ink
  statusOk: '#007a55',
  statusWarn: '#8a5a00',
  statusErr: '#b00030',
  border: '#0b1030',
} as const

export const FONTS = {
  brandGlyph: fontDataUrl('@fontsource/yuji-boku/files/yuji-boku-84-400-normal.woff2'),
  displayBold: fontDataUrl('@fontsource/chakra-petch/files/chakra-petch-latin-700-normal.woff2'),
  displaySemibold: fontDataUrl(
    '@fontsource/chakra-petch/files/chakra-petch-latin-600-normal.woff2',
  ),
  monoRegular: fontDataUrl(
    '@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2',
  ),
  monoBold: fontDataUrl('@fontsource/jetbrains-mono/files/jetbrains-mono-latin-700-normal.woff2'),
} as const

/** 尋 (hiro): the traditional unit for a fathom (src/theme/BrandMark.tsx). */
export const BRAND_GLYPH = '尋'

export const FONT_FACE_CSS = `
@font-face { font-family: 'Fathom Brand'; src: url('${FONTS.brandGlyph}') format('woff2'); font-weight: 400; unicode-range: U+5C0B; }
@font-face { font-family: 'Chakra Petch'; src: url('${FONTS.displaySemibold}') format('woff2'); font-weight: 600; }
@font-face { font-family: 'Chakra Petch'; src: url('${FONTS.displayBold}') format('woff2'); font-weight: 700; }
@font-face { font-family: 'JetBrains Mono'; src: url('${FONTS.monoRegular}') format('woff2'); font-weight: 400; }
@font-face { font-family: 'JetBrains Mono'; src: url('${FONTS.monoBold}') format('woff2'); font-weight: 700; }
`
