/**
 * ANSI colour for untrusted log lines.
 *
 * A log line is text from a process nobody vetted. It may colour itself, but it must not be able
 * to move the cursor, retitle the window, emit a hyperlink, hide text, or reorder what the
 * operator reads. So exactly one kind of escape sequence is interpreted: SGR (`ESC [ … m`, select
 * graphic rendition), and only its styling parameters. Every other sequence, every C0/C1 control
 * other than tab, DEL, and the bidi overrides and isolates are removed, byte-for-byte as
 * `stripAnsi` has always removed them. The result is plain text plus styled runs over it; the
 * caller renders the runs as React text, never as markup.
 *
 * Search and filtering work on `text`; the runs only say how to paint it.
 */

// CSI (`ESC [ … final`), OSC (`ESC ] … BEL|ST`), DCS/SOS/PM/APC strings, then any other two-byte
// escape, then the C1 CSI introducer. Built from code points so no literal control character sits
// in the source.
const ESC = String.fromCharCode(0x1b)
const BEL = String.fromCharCode(0x07)
const C1_CSI = String.fromCharCode(0x9b)
const SEQUENCES = new RegExp(
  [
    `${ESC}\\[[0-?]*[ -/]*[@-~]`,
    `${ESC}\\][^${BEL}${ESC}]*(?:${BEL}|${ESC}\\\\)?`,
    `${ESC}[PX^_][^${ESC}]*(?:${ESC}\\\\)?`,
    `${ESC}[@-_]`,
    `${C1_CSI}[0-?]*[ -/]*[@-~]`,
  ].join('|'),
  'g',
)

/** Any character `parseAnsi` would remove or interpret. A line without one is returned as is. */
// eslint-disable-next-line no-control-regex -- the point of the pattern is to find controls
const NEEDS_WORK = /[\u0000-\u0008\u000a-\u001f\u007f-\u009f‪-‮⁦-⁩]/

/** An SGR sequence: CSI, parameters of digits and separators only, final `m`. */
const SGR = new RegExp(`^(?:${ESC}\\[|${C1_CSI})([0-9;:]*)m$`)

/**
 * Parameter strings longer than this are not interpreted (the sequence is still removed). No
 * real program writes one; a hostile line could write a megabyte of `;`.
 */
export const SGR_MAX_PARAMS_LENGTH = 256

/**
 * A colour: 0–15 are the sixteen named colours (theme tokens); 16 and up are an RGB value
 * packed as `RGB_BASE + 0xRRGGBB`.
 */
export type AnsiColor = number
export const RGB_BASE = 0x1000000

export interface AnsiStyle {
  readonly fg: AnsiColor | null
  readonly bg: AnsiColor | null
  readonly bold: boolean
  readonly dim: boolean
  readonly italic: boolean
  readonly underline: boolean
  readonly inverse: boolean
  readonly strike: boolean
}

/** A styled stretch of the plain text, `[start, end)` in UTF-16 units. */
export interface AnsiRun {
  readonly start: number
  readonly end: number
  readonly style: AnsiStyle
}

export interface AnsiLine {
  /** The line with every control sequence and control character removed. */
  readonly text: string
  /** Styled runs over `text`, in order and non-overlapping. Unstyled text has no run. */
  readonly runs: readonly AnsiRun[]
}

const DEFAULT: AnsiStyle = Object.freeze({
  fg: null,
  bg: null,
  bold: false,
  dim: false,
  italic: false,
  underline: false,
  inverse: false,
  strike: false,
})

function isDefault(s: AnsiStyle): boolean {
  return (
    s.fg === null &&
    s.bg === null &&
    !s.bold &&
    !s.dim &&
    !s.italic &&
    !s.underline &&
    !s.inverse &&
    !s.strike
  )
}

/** C0 and C1 controls other than tab, plus DEL, plus bidi overrides and isolates. */
function isRemoved(code: number): boolean {
  return (
    (code < 0x20 && code !== 0x09) ||
    (code >= 0x7f && code < 0xa0) ||
    (code >= 0x202a && code <= 0x202e) ||
    (code >= 0x2066 && code <= 0x2069)
  )
}

function clean(segment: string): string {
  if (!NEEDS_WORK.test(segment)) return segment
  let out = ''
  for (const ch of segment) {
    if (!isRemoved(ch.codePointAt(0) ?? 0)) out += ch
  }
  return out
}

/** The xterm 256-colour palette entry `n` (16–255) as packed RGB. */
function palette256(n: number): AnsiColor {
  if (n < 16) return n
  if (n < 232) {
    const i = n - 16
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40)
    return rgb(level(Math.floor(i / 36)), level(Math.floor(i / 6) % 6), level(i % 6))
  }
  const g = 8 + (n - 232) * 10
  return rgb(g, g, g)
}

function rgb(r: number, g: number, b: number): AnsiColor {
  return RGB_BASE + ((r << 16) | (g << 8) | b)
}

const byte = (v: number | undefined): v is number =>
  v !== undefined && Number.isInteger(v) && v >= 0 && v <= 255

/** `38`/`48` followed by `5;n` or `2;r;g;b`, read from `nums` at `i`. Returns [colour, consumed]. */
function extended(nums: readonly number[], i: number): [AnsiColor | null, number] {
  const mode = nums[i]
  if (mode === 5) {
    const n = nums[i + 1]
    return [byte(n) ? palette256(n) : null, 2]
  }
  if (mode === 2) {
    const [r, g, b] = [nums[i + 1], nums[i + 2], nums[i + 3]]
    return [byte(r) && byte(g) && byte(b) ? rgb(r, g, b) : null, 4]
  }
  // Unknown or missing mode: the rest of the sequence cannot be read reliably.
  return [null, nums.length]
}

/** Colon sub-parameters (`38:2::r:g:b`, `38:2:r:g:b`, `38:5:n`) for one parameter. */
function extendedColon(parts: readonly string[]): AnsiColor | null {
  const nums = parts.map((p) => (p === '' ? undefined : Number(p)))
  if (nums[1] === 5) return byte(nums[2]) ? palette256(nums[2]) : null
  if (nums[1] === 2) {
    // With a colour-space id the channels start one later.
    const off = parts.length >= 6 ? 3 : 2
    const [r, g, b] = [nums[off], nums[off + 1], nums[off + 2]]
    return byte(r) && byte(g) && byte(b) ? rgb(r, g, b) : null
  }
  return null
}

type Mutable<T> = { -readonly [K in keyof T]: T[K] }

/** Apply one SGR parameter string to `cur`, returning the new style (or `cur` if unchanged). */
export function applySgr(cur: AnsiStyle, params: string): AnsiStyle {
  if (params.length > SGR_MAX_PARAMS_LENGTH) return cur
  const s: Mutable<AnsiStyle> = { ...cur }
  const fields = params.split(';')
  const nums: number[] = []
  for (const f of fields) {
    if (f.includes(':')) {
      // A colon parameter is self-contained; apply it in place.
      const parts = f.split(':')
      const head = Number(parts[0] === '' ? 0 : parts[0])
      if (head === 38 || head === 48) {
        const c = extendedColon(parts)
        if (c !== null) s[head === 38 ? 'fg' : 'bg'] = c
      } else if (head === 4) {
        s.underline = parts[1] !== '0'
      }
      nums.push(-1)
      continue
    }
    nums.push(f === '' ? 0 : Number(f))
  }
  for (let i = 0; i < nums.length; i++) {
    const n = nums[i] ?? -1
    if (n === 0) Object.assign(s, DEFAULT)
    else if (n === 1) s.bold = true
    else if (n === 2) s.dim = true
    else if (n === 3) s.italic = true
    else if (n === 4 || n === 21) s.underline = true
    else if (n === 7) s.inverse = true
    else if (n === 9) s.strike = true
    else if (n === 22) {
      s.bold = false
      s.dim = false
    } else if (n === 23) s.italic = false
    else if (n === 24) s.underline = false
    else if (n === 27) s.inverse = false
    else if (n === 29) s.strike = false
    else if (n >= 30 && n <= 37) s.fg = n - 30
    else if (n === 39) s.fg = null
    else if (n >= 40 && n <= 47) s.bg = n - 40
    else if (n === 49) s.bg = null
    else if (n >= 90 && n <= 97) s.fg = n - 90 + 8
    else if (n >= 100 && n <= 107) s.bg = n - 100 + 8
    else if (n === 38 || n === 48) {
      const [c, used] = extended(nums, i + 1)
      if (c !== null) s[n === 38 ? 'fg' : 'bg'] = c
      i += used
    }
    // 5/6 blink and 8 conceal are ignored on purpose: a log line may not hide text or flash.
  }
  return s.fg === cur.fg &&
    s.bg === cur.bg &&
    s.bold === cur.bold &&
    s.dim === cur.dim &&
    s.italic === cur.italic &&
    s.underline === cur.underline &&
    s.inverse === cur.inverse &&
    s.strike === cur.strike
    ? cur
    : isDefault(s)
      ? DEFAULT
      : Object.freeze(s)
}

/** Plain text and styled runs for one line. */
export function parseAnsi(line: string): AnsiLine {
  if (!NEEDS_WORK.test(line)) return { text: line, runs: [] }
  let text = ''
  const runs: AnsiRun[] = []
  let style = DEFAULT
  const emit = (segment: string) => {
    const part = clean(segment)
    if (part === '') return
    const start = text.length
    text += part
    if (style === DEFAULT) return
    const prev = runs[runs.length - 1]
    if (prev && prev.style === style && prev.end === start) {
      runs[runs.length - 1] = { start: prev.start, end: text.length, style }
    } else {
      runs.push({ start, end: text.length, style })
    }
  }
  let at = 0
  SEQUENCES.lastIndex = 0
  for (let m = SEQUENCES.exec(line); m !== null; m = SEQUENCES.exec(line)) {
    if (m.index > at) emit(line.slice(at, m.index))
    at = m.index + m[0].length
    const sgr = SGR.exec(m[0])
    if (sgr) style = applySgr(style, sgr[1] ?? '')
  }
  if (at < line.length) emit(line.slice(at))
  return { text, runs }
}

/** The line with every terminal control sequence and control character removed. */
export function stripAnsi(line: string): string {
  if (!NEEDS_WORK.test(line)) return line
  return clean(line.replace(SEQUENCES, ''))
}

/** The sixteen named colours, as theme tokens (`--ansi-<name>`). */
export const ANSI_NAMES = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'bright-black',
  'bright-red',
  'bright-green',
  'bright-yellow',
  'bright-blue',
  'bright-magenta',
  'bright-cyan',
  'bright-white',
] as const

/**
 * CSS for a colour. Named colours are theme tokens tuned for contrast in both themes; an RGB
 * colour keeps its hue but has its lightness clamped into the theme's readable band
 * (`--ansi-l-min`…`--ansi-l-max`), so `38;2;0;0;0` cannot paint black on the dark theme.
 */
export function ansiColorCss(c: AnsiColor): string {
  if (c < 16) return `var(--ansi-${ANSI_NAMES[c] ?? 'white'})`
  const v = c - RGB_BASE
  const r = (v >> 16) & 0xff
  const g = (v >> 8) & 0xff
  const b = v & 0xff
  return `oklch(from rgb(${String(r)} ${String(g)} ${String(b)}) clamp(var(--ansi-l-min, 0), l, var(--ansi-l-max, 1)) c h)`
}

/** Inline style for a run. Backgrounds are tinted, not solid, so the default text stays legible. */
export function ansiCss(s: AnsiStyle): Record<string, string | number> {
  const css: Record<string, string | number> = {}
  let fg = s.fg === null ? null : ansiColorCss(s.fg)
  let bg = s.bg === null ? null : `color-mix(in oklch, ${ansiColorCss(s.bg)} 35%, transparent)`
  if (s.inverse) {
    const solidBg = s.fg === null ? 'var(--foreground)' : ansiColorCss(s.fg)
    fg = s.bg === null ? 'var(--background)' : ansiColorCss(s.bg)
    bg = solidBg
  }
  if (fg !== null) css.color = fg
  if (bg !== null) css.backgroundColor = bg
  if (s.bold) css.fontWeight = 700
  if (s.dim) css.opacity = 0.7
  if (s.italic) css.fontStyle = 'italic'
  const deco = [s.underline ? 'underline' : '', s.strike ? 'line-through' : '']
    .filter(Boolean)
    .join(' ')
  if (deco) css.textDecorationLine = deco
  return css
}
