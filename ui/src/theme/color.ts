/**
 * Pure color math for the theme contract (contracts/theme.md, SC-008).
 *
 * Used by the contrast test to evaluate `tokens.css`, and by tooling that converts the
 * design-intent hex values into the OKLCH values the stylesheet ships.
 */

/** Linear-light-free sRGB triple in 0..1 plus alpha in 0..1. */
export interface Rgba {
  r: number
  g: number
  b: number
  a: number
}

export interface Oklch {
  l: number
  c: number
  h: number
  a: number
}

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v))

const toLinear = (v: number): number => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)

const toGamma = (v: number): number => (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055)

export function hexToRgba(hex: string): Rgba {
  const m = /^#?([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(hex.trim())
  if (!m?.[1]) throw new Error(`invalid hex color: ${hex}`)
  const n = Number.parseInt(m[1], 16)
  return {
    r: ((n >> 16) & 0xff) / 255,
    g: ((n >> 8) & 0xff) / 255,
    b: (n & 0xff) / 255,
    a: m[2] ? Number.parseInt(m[2], 16) / 255 : 1,
  }
}

export function oklchToRgba({ l, c, h, a }: Oklch): Rgba {
  const hr = (h * Math.PI) / 180
  const A = c * Math.cos(hr)
  const B = c * Math.sin(hr)

  const l_ = l + 0.3963377774 * A + 0.2158037573 * B
  const m_ = l - 0.1055613458 * A - 0.0638541728 * B
  const s_ = l - 0.0894841775 * A - 1.291485548 * B

  const L = l_ ** 3
  const M = m_ ** 3
  const S = s_ ** 3

  const r = 4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S
  const g = -1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S
  const b = -0.0041960863 * L - 0.7034186147 * M + 1.707614701 * S

  return { r: clamp01(toGamma(r)), g: clamp01(toGamma(g)), b: clamp01(toGamma(b)), a }
}

export function rgbaToOklch({ r, g, b, a }: Rgba): Oklch {
  const lr = toLinear(r)
  const lg = toLinear(g)
  const lb = toLinear(b)

  const L = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb)
  const M = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb)
  const S = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb)

  const l = 0.2104542553 * L + 0.793617785 * M - 0.0040720468 * S
  const A = 1.9779984951 * L - 2.428592205 * M + 0.4505937099 * S
  const B = 0.0259040371 * L + 0.7827717662 * M - 0.808675766 * S

  const c = Math.hypot(A, B)
  const h = c < 1e-4 ? 0 : ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360
  return { l, c, h, a }
}

const OKLCH_RE =
  /^oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)(?:deg)?\s*(?:\/\s*([\d.]+)(%?))?\s*\)$/i

/** Parse `#rrggbb[aa]` or `oklch(L C H [/ A])`. Returns undefined for anything else. */
export function parseColor(value: string): Rgba | undefined {
  const v = value.trim()
  if (v.startsWith('#')) return hexToRgba(v)
  const m = OKLCH_RE.exec(v)
  if (!m) return undefined
  const [, l, lPct, c, h, a, aPct] = m
  return oklchToRgba({
    l: Number(l) / (lPct ? 100 : 1),
    c: Number(c),
    h: Number(h),
    a: a === undefined ? 1 : Number(a) / (aPct ? 100 : 1),
  })
}

export function formatOklch({ l, c, h, a }: Oklch): string {
  const base = `${(l * 100).toFixed(2)}% ${c.toFixed(4)} ${h.toFixed(2)}`
  return a < 1 ? `oklch(${base} / ${Math.round(a * 100)}%)` : `oklch(${base})`
}

/** Source-over composite of `fg` onto an opaque `bg`. */
export function composite(fg: Rgba, bg: Rgba): Rgba {
  const a = fg.a
  return {
    r: fg.r * a + bg.r * (1 - a),
    g: fg.g * a + bg.g * (1 - a),
    b: fg.b * a + bg.b * (1 - a),
    a: 1,
  }
}

export function relativeLuminance({ r, g, b }: Rgba): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b)
}

/** WCAG 2.x contrast ratio; translucent foregrounds are composited over `bg` first. */
export function contrastRatio(fg: Rgba, bg: Rgba): number {
  const f = relativeLuminance(fg.a < 1 ? composite(fg, bg) : fg)
  const b = relativeLuminance(bg)
  const [hi, lo] = f > b ? [f, b] : [b, f]
  return (hi + 0.05) / (lo + 0.05)
}
