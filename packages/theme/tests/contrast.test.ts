// @vitest-environment node
/**
 * SC-008: WCAG 2.x contrast for every text/background pair in every theme variant.
 *
 * Parses tokens.css directly (no browser), cascades the variant's rule blocks, resolves
 * `var()` references, and checks the ratios from contracts/theme.md § Verification. A second suite
 * pins each token's hue to the contract's design intent: tuning may move lightness only.
 */
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { contrastRatio, hexToRgba, parseColor, rgbaToOklch, type Rgba } from '../src/color'

const css = readFileSync(fileURLToPath(new URL('../src/tokens.css', import.meta.url)), 'utf8')

/** selector → declarations, in source order, for plain rule blocks (at-rules skipped). */
function parseBlocks(source: string): Map<string, Map<string, string>> {
  const blocks = new Map<string, Map<string, string>>()
  const stripped = source.replace(/\/\*[\s\S]*?\*\//g, '')
  for (const m of stripped.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = (m[1] ?? '').trim()
    if (selector.startsWith('@')) continue
    const decls = blocks.get(selector) ?? new Map<string, string>()
    for (const decl of (m[2] ?? '').split(';')) {
      const idx = decl.indexOf(':')
      if (idx < 0) continue
      const name = decl.slice(0, idx).trim()
      if (name.startsWith('--')) decls.set(name, decl.slice(idx + 1).trim())
    }
    blocks.set(selector, decls)
  }
  return blocks
}

const blocks = parseBlocks(css)

const VARIANTS = {
  'dark-cyan': [':root', '.dark'],
  'dark-magenta': [':root', '.dark', ".dark[data-accent='magenta']"],
  light: [':root'],
  'light-magenta': [':root', ":root:not(.dark)[data-accent='magenta']"],
} as const satisfies Record<string, readonly string[]>

type Variant = keyof typeof VARIANTS

function tokensFor(variant: Variant): Map<string, string> {
  const merged = new Map<string, string>()
  for (const selector of VARIANTS[variant]) {
    const decls = blocks.get(selector)
    if (!decls) throw new Error(`tokens.css has no block for selector ${selector}`)
    for (const [k, v] of decls) merged.set(k, v)
  }
  return merged
}

/** Follow `var(--x)` chains to the declared literal. */
function resolveRaw(tokens: Map<string, string>, name: string, depth = 0): string {
  if (depth > 10) throw new Error(`var() cycle at ${name}`)
  const raw = tokens.get(name)
  if (raw === undefined) throw new Error(`token ${name} is undefined`)
  const ref = /^var\((--[\w-]+)\)$/.exec(raw)
  return ref?.[1] ? resolveRaw(tokens, ref[1], depth + 1) : raw
}

function resolve(tokens: Map<string, string>, name: string): Rgba {
  const raw = resolveRaw(tokens, name)
  const color = parseColor(raw)
  if (!color) throw new Error(`token ${name} is not a parseable color: ${raw}`)
  return color
}

const TEXT = [
  '--foreground',
  '--muted-foreground',
  '--accent-neon',
  '--accent-2',
  '--status-ok',
  '--status-warn',
  '--status-err',
  '--destructive',
] as const
const SURFACES = ['--background', '--card'] as const
const TEXT_ON_OWN_SURFACE = [
  ['--primary-foreground', '--primary'],
  ['--card-foreground', '--card'],
  ['--popover-foreground', '--popover'],
  ['--muted-foreground', '--muted'],
  ['--accent-foreground', '--accent'],
  ['--secondary-foreground', '--secondary'],
  // Tooltips render inverse (bg-foreground text-background).
  ['--background', '--foreground'],
] as const
const NON_TEXT = [
  '--border',
  '--chart-1',
  '--chart-2',
  '--chart-3',
  '--chart-4',
  '--chart-5',
] as const
/** Tokens used for display text (≥ 18px headings, pane titles, brand). */
const DISPLAY = ['--foreground', '--accent-neon', '--accent-2', '--muted-foreground'] as const

const AA_TEXT = 4.5
const AA_NON_TEXT = 3

describe.each(Object.keys(VARIANTS) as Variant[])('contrast: %s', (variant) => {
  const tokens = tokensFor(variant)
  const ratio = (fg: string, bg: string) => contrastRatio(resolve(tokens, fg), resolve(tokens, bg))

  for (const fg of TEXT) {
    for (const bg of SURFACES) {
      it(`${fg} on ${bg} ≥ ${AA_TEXT}:1`, () => {
        expect(ratio(fg, bg)).toBeGreaterThanOrEqual(AA_TEXT)
      })
    }
  }

  for (const [fg, bg] of TEXT_ON_OWN_SURFACE) {
    it(`${fg} on ${bg} ≥ ${AA_TEXT}:1`, () => {
      expect(ratio(fg, bg)).toBeGreaterThanOrEqual(AA_TEXT)
    })
  }

  for (const fg of [...NON_TEXT, ...DISPLAY]) {
    for (const bg of SURFACES) {
      it(`${fg} on ${bg} ≥ ${AA_NON_TEXT}:1 (non-text / large display)`, () => {
        expect(ratio(fg, bg)).toBeGreaterThanOrEqual(AA_NON_TEXT)
      })
    }
  }

  it('uses square corners', () => {
    expect(tokens.get('--radius')).toBe('0')
  })
})

/** Contract design intent (contracts/theme.md). Lightness may be tuned; hue may not. */
const INTENT: Record<Variant, Record<string, string>> = {
  'dark-cyan': {
    '--background': '#070b1a',
    '--card': '#0c1228',
    '--foreground': '#d6f6ff',
    '--muted-foreground': '#8a9cc9',
    '--accent-neon': '#3ef2ff',
    '--accent-2': '#ff2bd6',
    '--border': '#8b6cff',
    '--destructive': '#ff4d6d',
    '--status-ok': '#39ffb0',
    '--status-warn': '#ffd23f',
    '--status-err': '#ff4d6d',
  },
  'dark-magenta': { '--accent-neon': '#ff2bd6', '--accent-2': '#3ef2ff' },
  light: {
    '--background': '#eef1fb',
    '--foreground': '#0b1030',
    '--muted-foreground': '#4a5580',
    '--accent-neon': '#006a80',
    '--accent-2': '#a0107f',
    '--border': '#0b1030',
    '--destructive': '#b00030',
    '--status-ok': '#007a55',
    '--status-warn': '#8a5a00',
    '--status-err': '#b00030',
  },
  'light-magenta': { '--accent-neon': '#a0107f', '--accent-2': '#006a80' },
}

const hueDistance = (a: number, b: number) => {
  const d = Math.abs(a - b) % 360
  return d > 180 ? 360 - d : d
}

describe.each(Object.keys(INTENT) as Variant[])('hue lock: %s', (variant) => {
  const tokens = tokensFor(variant)
  for (const [token, hex] of Object.entries(INTENT[variant])) {
    it(`${token} keeps the hue of ${hex}`, () => {
      const declared = /^oklch\(\s*[\d.]+%?\s+[\d.]+\s+([\d.]+)/.exec(resolveRaw(tokens, token))
      expect(declared, `${token} must be declared in oklch()`).not.toBeNull()
      const intent = rgbaToOklch(hexToRgba(hex))
      // Declared values are rounded to 2 decimals; anything under 0.5° is the same hue.
      expect(hueDistance(Number(declared?.[1]), intent.h)).toBeLessThan(0.5)
    })
  }
})

/**
 * SC-008 (utilities): a Tailwind alpha modifier MULTIPLIES with the token's own alpha.
 *
 * `--border` is already declared at 55% in dark, so `border-border/60` renders at 33% effective
 * and composites to 1.90:1 on `--card` — under the 3:1 floor SC 1.4.11 sets for any boundary that
 * is required to identify a control. The suites above resolve tokens and so cannot see this: the
 * modifier lives in a class name, not in tokens.css. This suite reads the class names.
 *
 * Three things are enforced:
 *   1. the utility names a token that exists (a typo generates no CSS at all, silently);
 *   2. a control boundary is never thinned, because it is the only thing identifying the control;
 *   3. anything else that lands under 3:1 is recorded as a decorative seam, with a reason.
 */
/**
 * Every frontend source tree in the workspace that renders with these tokens: the shared packages
 * and each app. A tree that does not exist yet (an app not scaffolded) is skipped.
 */
const SRC_DIRS = [
  '../../theme/src',
  '../../ui/src',
  '../../terminal/src',
  '../../../ui/src',
  '../../../apps/depths/web/src',
]
  .map((rel) => fileURLToPath(new URL(rel, import.meta.url)))
  .filter((dir) => existsSync(dir))

function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name)
    if (entry.isDirectory()) out.push(...sourceFiles(full))
    else if (/\.tsx?$/.test(entry.name)) out.push(full)
  }
  return out
}

/** `dark:border-b-border/60` → name `border`, alpha 60. */
const BORDER_UTILITY = /\bborder-(?:[trblxyse]-)?([a-z][a-z0-9-]*?)\/(\d{1,3})\b/g

/** Surfaces a boundary can land on. `--muted` is included here even though SURFACES omits it. */
const BOUNDARY_SURFACES = ['--background', '--card', '--muted'] as const

/** Tailwind's own palette scales. Not design tokens; they cannot follow a theme or accent swap. */
const RAW_PALETTE =
  /^(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}$/

/**
 * Raw-palette borders that predate the token system. Listed so they are counted rather than
 * invisible; each should become a status token. Adding to this list is a regression.
 */
const RAW_PALETTE_DEBT: Record<string, string> = {
  'amber-600/40': 'namespaceAccess + KubeconfigExportDialog warning callouts',
  'amber-500/60': 'fleet Degraded / Reconnecting state',
  'emerald-500/50': 'fleet Healthy / Live state',
  'red-500/60': 'fleet Unreachable state',
  'fuchsia-500/60': 'fleet AuthError state',
  'sky-500/50': 'fleet Pending coverage state (046)',
}

/** Boundaries that identify a control. These may not carry an alpha modifier at all. */
const CONTROL_TOKENS = new Set(['input', 'control-border', 'ring'])

/**
 * Tokens whose borders are decorative: a divider, a panel edge, or an emphasis rule around a
 * region that a tinted fill and its own label already identify. SC 1.4.11 scopes the 3:1 floor to
 * a boundary *required* to identify a control, so these may be thinned.
 *
 * Keyed by token, not by token/alpha, so the list stays readable — and a token that is not listed
 * here and is not a control token must clear 3:1 at every alpha it is used with. The point of the
 * list is that a new alpha-thinned border forces someone to decide which kind it is.
 */
const DECORATIVE_SEAM_TOKENS: Record<string, string> = {
  border: 'panel edges, dividers and rules; never the sole boundary of a control',
  destructive: 'edge of a destructive callout, paired with a tinted fill and its own label',
  'status-err': 'edge of an error callout, paired with a tinted fill and its own label',
  'status-warn': 'edge of a warning callout, paired with a tinted fill and its own label',
  'accent-neon': 'emphasis edge on an already-labelled region; the focus ring is --ring, not this',
}

describe.each(Object.keys(VARIANTS) as Variant[])('contrast: %s border utilities', (variant) => {
  const tokens = tokensFor(variant)
  const files = SRC_DIRS.flatMap(sourceFiles)

  /** token name → alpha modifiers it is used with, anywhere in the workspace's frontend sources. */
  const used = new Map<string, Set<number>>()
  for (const file of files) {
    for (const m of readFileSync(file, 'utf8').matchAll(BORDER_UTILITY)) {
      const name = m[1]
      const alpha = Number(m[2])
      if (!name || !Number.isFinite(alpha)) continue
      const seen = used.get(name) ?? new Set<number>()
      seen.add(alpha)
      used.set(name, seen)
    }
  }

  for (const [name, alphas] of [...used].sort(([a], [b]) => a.localeCompare(b))) {
    if (RAW_PALETTE.test(name)) {
      for (const alpha of alphas) {
        it(`border-${name}/${alpha} is recorded palette debt`, () => {
          expect(RAW_PALETTE_DEBT).toHaveProperty(`${name}/${alpha}`)
        })
      }
      continue
    }

    it(`border-${name}/… names a real token`, () => {
      // A name with no token generates no CSS, so the border simply does not render.
      expect(tokens.has(`--color-${name}`) || tokens.has(`--${name}`)).toBe(true)
    })

    if (!tokens.has(`--color-${name}`) && !tokens.has(`--${name}`)) continue

    if (CONTROL_TOKENS.has(name)) {
      it(`border-${name} is never thinned by a modifier`, () => {
        // It is the only boundary the control has, so a modifier would be all of its contrast.
        expect([...alphas].sort((a, b) => a - b)).toEqual([])
      })
      continue
    }

    if (name in DECORATIVE_SEAM_TOKENS) {
      it(`border-${name} is a declared decorative seam`, () => {
        expect(DECORATIVE_SEAM_TOKENS[name]).toBeTruthy()
      })
      continue
    }

    for (const alpha of [...alphas].sort((a, b) => a - b)) {
      const base = resolve(tokens, tokens.has(`--${name}`) ? `--${name}` : `--color-${name}`)
      const thinned: Rgba = { ...base, a: base.a * (alpha / 100) }
      for (const surface of BOUNDARY_SURFACES) {
        it(`border-${name}/${alpha} on ${surface} ≥ ${AA_NON_TEXT}:1`, () => {
          const r = contrastRatio(thinned, resolve(tokens, surface))
          expect(
            r,
            `border-${name}/${alpha} composites to ${r.toFixed(2)}:1 on ${surface}. Either use an ` +
              `opaque control token, or declare --${name} a decorative seam with a reason.`,
          ).toBeGreaterThanOrEqual(AA_NON_TEXT)
        })
      }
    }
  }
})
