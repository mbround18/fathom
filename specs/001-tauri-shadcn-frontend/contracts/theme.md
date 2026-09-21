# Contract: Neon-Noir Terminal Theme Tokens

**Plan**: [../plan.md](../plan.md) · **Decision**: research R8 · **Spec**: FR-018, FR-019, FR-021, SC-008

## Direction

Terminal structure, neon light. Data is laid out like a terminal: monospace, dense, square, box-drawn, with inverse-video selection. The light comes from neon signage in a deep-water night: cyan, magenta, and violet on an abyssal navy.

The brand mark is **尋** (*hiro*), the traditional Japanese unit for a fathom, in brush lettering, followed by `FATHOM` set in the angular display face.

## Token set

The theme uses shadcn's standard variables, so generated components are themed without edits, plus the Fathom extensions below. Tokens are defined in `ui/src/theme/tokens.css` in OKLCH and mapped into Tailwind v4 with `@theme inline`.

**Variants**:
- `.dark` (the default) is neon-noir.
- `:root` without `.dark` is the light "blueprint" variant.
- The accent comes from `html[data-accent="cyan" | "magenta"]`, with cyan as the default.

| Token | Dark / cyan | Dark / magenta | Light (blueprint) |
|---|---|---|---|
| `--background` | `#070b1a` | `#070b1a` | `#eef1fb` |
| `--card` / `--popover` / `--sidebar` | `#0c1228` | `#0c1228` | `#ffffff` |
| `--foreground` | `#d6f6ff` (ice) | `#d6f6ff` | `#0b1030` |
| `--muted-foreground` | `#8a9cc9` | `#8a9cc9` | `#4a5580` |
| `--accent-neon` (identifiers, links, active nav) | `#3ef2ff` | `#ff2bd6` | `#006a80` / `#a0107f` |
| `--accent-2` (the other neon, for secondary highlights) | `#ff2bd6` | `#3ef2ff` | `#a0107f` / `#006a80` |
| `--border` (structure) | `#8b6cff` at 55% | same | `#0b1030` |
| `--primary` (selection, inverse video) | `--accent-neon` bg / `#070b1a` fg | same | `--accent-neon` bg / `#ffffff` fg |
| `--ring` (focus) | `--accent-neon` | `--accent-neon` | `--accent-neon` |
| `--destructive` | `#ff4d6d` | `#ff4d6d` | `#b00030` |
| `--status-ok` / `--status-warn` / `--status-err` | `#39ffb0` / `#ffd23f` / `#ff4d6d` | same | `#007a55` / `#8a5a00` / `#b00030` |
| `--chart-1..5` | cyan, magenta, violet, mint, amber, each ≥ 3:1 against the background | same | ink-darkened equivalents |
| `--radius` | `0` | `0` | `0` |
| `--font-mono` (body, tables, identifiers, logs, YAML) | JetBrains Mono | same | same |
| `--font-sans` | JetBrains Mono. The UI is monospace-first; there is no separate sans. | same | same |
| `--font-display` (headings, pane titles, top bar, buttons ≥ 14px uppercase) | Chakra Petch 600/700 | same | same |
| `--font-brand` (the 尋 mark only) | Yuji Boku (brush), with Noto Serif JP as fallback | same | same |
| `--glow` | `0 0 6px color-mix(in oklch, var(--accent-neon) 55%, transparent)` | same formula | `none` |
| `--scanline-opacity` | `0.05` | `0.05` | `0` |
| `--grain-opacity` (the worn, lived-in noise) | `0.06` | `0.06` | `0.03` |

The hex values above are the design intent. The shipped file stores their OKLCH equivalents. T041 may adjust lightness so the contrast checks pass, but the hue stays fixed.

## Effects (decorative only)

| Effect | Where | Mechanism |
|---|---|---|
| Neon glow | Brand mark, active nav item, pane titles, focused pane border | `text-shadow` / `box-shadow: var(--glow)` |
| Scanlines | Top bar, status bar | `repeating-linear-gradient` overlay (`::after`, `pointer-events: none`) |
| Grain / wear | The whole app background, beneath content | One static SVG `feTurbulence` noise tile as a background-image. It never animates. |
| Glitch (RGB split plus a slice jitter) | Brand mark on hover, context switch transition (~200 ms), error banners | CSS keyframes on `text-shadow` / `clip-path`. **Only when `data-glitch="on"` and effects are on.** |
| Circuit traces | Empty states, error states, the first-run screen | Inline SVG line art using `--border` / `--accent-neon` |

## Rules

1. **Status color never stands alone.** Every status also shows a glyph or text (`●`, `▲`, `✖`, or the word itself).
2. **Effects are applied only to shell chrome, transitions, and empty/error states**, never to table rows, log lines, or YAML.
3. **Effects are off when any of these is true:**
   - `@media (prefers-reduced-motion: reduce)` (this disables animations; the static grain and scanlines may remain),
   - `html[data-effects="off"]` (the settings toggle; this disables everything),
   - the light variant (glow and scanlines are off; grain is reduced).
   - Glitch animation defaults to off.
4. **Selection** is inverse video (`--primary`). **Keyboard focus** is a 1px `--ring` outline plus a leading block cursor `▌` in `--accent-neon` on the focused row.
5. **Borders** are 1px solid `--border`. Dividers use box-drawing glyphs only in decorative chrome, and those glyphs carry `aria-hidden`.
6. **Kanji is decoration plus brand only.** The mark is 尋. Section headers MAY carry a small, muted kanji annotation, `aria-hidden`: 概 overview, 器 pods, 網 network, 蔵 storage, 鍵 access, 設 settings. Text alternatives MUST remain English.
7. **Fonts are local.** Fonts are bundled through `@fontsource`, with the unicode-range subsets it ships, so only the needed glyph chunks load. Nothing is fetched from a CDN.

## Verification (SC-008)

`ui/tests/unit/contrast.test.ts` parses `tokens.css` and computes WCAG 2.x contrast for every text/background pair in the three variants (dark-cyan, dark-magenta, light):
- foreground, muted, accent-neon, accent-2, and status colors on `--background` and on `--card`;
- `--primary-foreground` on `--primary`.

It fails below 4.5:1 for text, and below 3:1 for `--border`, charts, and display text of 18px or larger.
