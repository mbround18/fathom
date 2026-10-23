/**
 * Depths' brand, as @companion/theme's BrandMark and loadFonts take it.
 *
 * 淵 (fuchi) is a deep pool, the depths of the water. The brand only renders that glyph (U+6DF5),
 * so register just the Yuji Boku fontsource chunk that contains it. If a fontsource update
 * renumbers its chunks this import fails the build rather than silently falling back.
 */
import type { BrandFont } from '@companion/theme'
import brandGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-72-400-normal.woff2?url'

export const BRAND_NAME = 'Depths'

export const BRAND_GLYPH = '淵'

export const BRAND_FONT: BrandFont = { url: brandGlyphUrl, unicodeRange: 'U+6DF5' }
