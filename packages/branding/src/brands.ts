import type { BrandFont } from '@companion/theme'

import keelGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-21-400-normal.woff2?url'
import ballastGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-61-400-normal.woff2?url'
import depthsGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-72-400-normal.woff2?url'
import portalGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-77-400-normal.woff2?url'
import fathomGlyphUrl from '@fontsource/yuji-boku/files/yuji-boku-84-400-normal.woff2?url'

export type SuiteBrandId = 'fathom' | 'depths' | 'keel' | 'ballast' | 'portal'

export interface SuiteBrand {
  id: SuiteBrandId
  name: string
  glyph: string
  unicodeRange: string
  story: string
  font: BrandFont
}

const FATHOM: SuiteBrand = {
  id: 'fathom',
  name: 'Fathom',
  glyph: '尋',
  unicodeRange: 'U+5C0B',
  story: '尋: seek and measure depth.',
  font: { url: fathomGlyphUrl, unicodeRange: 'U+5C0B' },
}

const DEPTHS: SuiteBrand = {
  id: 'depths',
  name: 'Depths',
  glyph: '淵',
  unicodeRange: 'U+6DF5',
  story: '淵: a deep pool; still depth.',
  font: { url: depthsGlyphUrl, unicodeRange: 'U+6DF5' },
}

const KEEL: SuiteBrand = {
  id: 'keel',
  name: 'Keel',
  glyph: '舵',
  unicodeRange: 'U+8235',
  story: '舵: rudder and helm; guided control.',
  font: { url: keelGlyphUrl, unicodeRange: 'U+8235' },
}

const BALLAST: SuiteBrand = {
  id: 'ballast',
  name: 'Ballast',
  glyph: '衡',
  unicodeRange: 'U+8861',
  story: '衡: balance and stability; trim under change.',
  font: { url: ballastGlyphUrl, unicodeRange: 'U+8861' },
}

const PORTAL: SuiteBrand = {
  id: 'portal',
  name: 'UwU Squad',
  glyph: '澪',
  unicodeRange: 'U+6F84',
  story: '澪: the marked channel ships follow between harbors.',
  font: { url: portalGlyphUrl, unicodeRange: 'U+6F84' },
}

export const SUITE_BRANDS: Readonly<Record<SuiteBrandId, SuiteBrand>> = {
  fathom: FATHOM,
  depths: DEPTHS,
  keel: KEEL,
  ballast: BALLAST,
  portal: PORTAL,
}

export function getSuiteBrand(id: SuiteBrandId): SuiteBrand {
  return SUITE_BRANDS[id]
}
