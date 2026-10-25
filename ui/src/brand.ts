/**
 * Fathom's brand, as @companion/theme's BrandMark and loadFonts take it.
 *
 * 尋 (hiro) is the traditional Japanese unit for a fathom. The brand only ever renders that glyph
 * (U+5C0B), so register just the Yuji Boku fontsource chunk that contains it. If a fontsource
 * update renumbers its chunks this import fails the build, rather than silently falling back to
 * the serif stack.
 */
import type { BrandFont } from '@companion/theme'
import brandGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-84-400-normal.woff2?url'

export const BRAND_NAME = 'Fathom'

export const BRAND_GLYPH = '尋'

export const BRAND_FONT: BrandFont = { url: brandGlyphUrl, unicodeRange: 'U+5C0B' }
