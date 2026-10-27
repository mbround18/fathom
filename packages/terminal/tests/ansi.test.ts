// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { ansiColorCss, ansiCss, parseAnsi, RGB_BASE, SGR_MAX_PARAMS_LENGTH, stripAnsi, type AnsiStyle } from '../src/ansi'

const E = '\u001b'

/** `stripAnsi` as Depths shipped it before colour (apps/depths/web logtext.ts): the reference. */
const ESC = String.fromCharCode(0x1b)
const BEL = String.fromCharCode(0x07)
const CSI = String.fromCharCode(0x9b)
const REF_SEQUENCES = new RegExp(
  [
    `${ESC}\\[[0-?]*[ -/]*[@-~]`,
    `${ESC}\\][^${BEL}${ESC}]*(?:${BEL}|${ESC}\\\\)?`,
    `${ESC}[PX^_][^${ESC}]*(?:${ESC}\\\\)?`,
    `${ESC}[@-_]`,
    `${CSI}[0-?]*[ -/]*[@-~]`,
  ].join('|'),
  'g',
)
function referenceStrip(line: string): string {
  const noSeq = line.replace(REF_SEQUENCES, '')
  let out = ''
  for (const ch of noSeq) {
    const code = ch.codePointAt(0) ?? 0
    if ((code < 0x20 && code !== 0x09) || (code >= 0x7f && code < 0xa0) || (code >= 0x202a && code <= 0x202e) || (code >= 0x2066 && code <= 0x2069)) continue
    out += ch
  }
  return out
}

/** The style of the character at `offset` (null when unstyled). */
function styleAt(line: string, offset: number): AnsiStyle | null {
  const { runs } = parseAnsi(line)
  return runs.find((r) => r.start <= offset && offset < r.end)?.style ?? null
}

describe('parseAnsi: text', () => {
  it('returns a plain line unchanged and unstyled', () => {
    expect(parseAnsi('plain text\twith tab')).toEqual({ text: 'plain text\twith tab', runs: [] })
  })

  it('removes non-SGR sequences exactly as stripAnsi always did', () => {
    const line = `${E}[31mred${E}[0m ${E}]0;title\u0007ok\u0000\u0008\tend`
    expect(parseAnsi(line).text).toBe('red ok\tend')
    expect(stripAnsi(line)).toBe('red ok\tend')
  })

  it('removes cursor moves, erases, OSC hyperlinks and titles, DCS, C1 and bidi', () => {
    const cases: [string, string][] = [
      [`a${E}[2Jb${E}[10;20Hc${E}[Kd`, 'abcd'],
      [`${E}]8;;https://evil.example${E}\\click${E}]8;;${E}\\`, 'click'],
      [`${E}]2;pwned\u0007x`, 'x'],
      [`${E}Pq#0;2;0;0;0${E}\\after`, 'after'],
      [`${E}_apc payload${E}\\z`, 'z'],
      [`a\u009b31mb\u0085c`, 'abc'],
      ['a‮b⁦c⁩d', 'abcd'],
      [`${E}[?25l${E}[?1049hhidden?`, 'hidden?'],
      // ESC 7 / ESC 8 sit outside ESC @–_: the ESC goes, the digit stays (as stripAnsi always did).
      [`x${E}7y${E}8z`, 'x7y8z'],
      ['line\r\nnext', 'linenext'],
    ]
    for (const [input, want] of cases) {
      expect(parseAnsi(input).text, JSON.stringify(input)).toBe(want)
      expect(stripAnsi(input)).toBe(want)
    }
  })

  it('agrees with the reference stripAnsi on random hostile input', () => {
    // Deterministic LCG so a failure reproduces.
    let seed = 0x5eed
    const rand = (n: number) => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff
      return seed % n
    }
    const alphabet = [E, '[', ']', 'P', '\\', '\u0007', '\u009b', ';', ':', '3', '8', '5', '2', 'm', 'H', 'a', ' ', '‮', '\u0000', '?', '1', 'é', '😀', '\u0085', 'X']
    for (let i = 0; i < 3_000; i++) {
      let s = ''
      const len = rand(40)
      for (let j = 0; j < len; j++) s += alphabet[rand(alphabet.length)] ?? ''
      expect(parseAnsi(s).text, JSON.stringify(s)).toBe(referenceStrip(s))
      expect(stripAnsi(s)).toBe(referenceStrip(s))
    }
  })

  it('never lets markup through as anything but text', () => {
    const { text } = parseAnsi(`${E}[1m<img src=x onerror=alert(1)>${E}[0m`)
    expect(text).toBe('<img src=x onerror=alert(1)>')
  })
})

describe('parseAnsi: SGR', () => {
  it('styles the 8 basic and 8 bright colours, foreground and background', () => {
    for (let n = 0; n < 8; n++) {
      expect(styleAt(`${E}[${String(30 + n)}mx`, 0)?.fg).toBe(n)
      expect(styleAt(`${E}[${String(90 + n)}mx`, 0)?.fg).toBe(8 + n)
      expect(styleAt(`${E}[${String(40 + n)}mx`, 0)?.bg).toBe(n)
      expect(styleAt(`${E}[${String(100 + n)}mx`, 0)?.bg).toBe(8 + n)
    }
  })

  it('reads bold, dim, italic, underline, inverse, strike and their resets', () => {
    const s = styleAt(`${E}[1;2;3;4;7;9mx`, 0)
    expect(s).toMatchObject({ bold: true, dim: true, italic: true, underline: true, inverse: true, strike: true })
    expect(styleAt(`${E}[1;2;3;4;7;9m${E}[22;23;24;27;29mx`, 0)).toBeNull()
  })

  it('splits runs at every change and merges equal neighbours', () => {
    const { text, runs } = parseAnsi(`a${E}[31mb${E}[31mc${E}[1md${E}[0me`)
    expect(text).toBe('abcde')
    expect(runs.map((r) => [r.start, r.end, r.style.fg, r.style.bold])).toEqual([
      [1, 3, 1, false],
      [3, 4, 1, true],
    ])
  })

  it('treats an empty SGR and 0 as a full reset, and 39/49 as colour resets', () => {
    expect(styleAt(`${E}[31;1m${E}[mx`, 0)).toBeNull()
    expect(styleAt(`${E}[31;42m${E}[39mx`, 0)).toMatchObject({ fg: null, bg: 2 })
    expect(styleAt(`${E}[31;42m${E}[49mx`, 0)).toMatchObject({ fg: 1, bg: null })
  })

  it('handles nested and repeated resets', () => {
    const { text, runs } = parseAnsi(`${E}[0;0;0m${E}[31m${E}[0m${E}[0mplain${E}[1m${E}[0;31;0mx`)
    expect(text).toBe('plainx')
    expect(runs).toEqual([])
  })

  it('reads 256-colour: named range, cube and grey ramp', () => {
    expect(styleAt(`${E}[38;5;9mx`, 0)?.fg).toBe(9)
    expect(styleAt(`${E}[38;5;196mx`, 0)?.fg).toBe(RGB_BASE + 0xff0000)
    expect(styleAt(`${E}[48;5;232mx`, 0)?.bg).toBe(RGB_BASE + 0x080808)
    expect(styleAt(`${E}[38:5:46mx`, 0)?.fg).toBe(RGB_BASE + 0x00ff00)
  })

  it('reads truecolor with semicolons and with colons (with and without a colour space)', () => {
    expect(styleAt(`${E}[38;2;1;2;3mx`, 0)?.fg).toBe(RGB_BASE + 0x010203)
    expect(styleAt(`${E}[48;2;10;20;30mx`, 0)?.bg).toBe(RGB_BASE + 0x0a141e)
    expect(styleAt(`${E}[38:2::1:2:3mx`, 0)?.fg).toBe(RGB_BASE + 0x010203)
    expect(styleAt(`${E}[38:2:1:2:3mx`, 0)?.fg).toBe(RGB_BASE + 0x010203)
  })

  it('keeps reading parameters after an extended colour', () => {
    expect(styleAt(`${E}[38;2;1;2;3;1;4mx`, 0)).toMatchObject({ fg: RGB_BASE + 0x010203, bold: true, underline: true })
  })

  it('ignores out-of-range and truncated extended colours', () => {
    expect(styleAt(`${E}[38;5;256mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38;2;300;0;0mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38;5mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38;2;1mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38;9;1;2;3mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38mx`, 0)).toBeNull()
  })

  it('does not hide or blink text (8, 5, 6 are ignored)', () => {
    expect(styleAt(`${E}[8mx`, 0)).toBeNull()
    expect(styleAt(`${E}[5;6mx`, 0)).toBeNull()
  })

  it('does not treat private or intermediate CSI ...m as SGR', () => {
    expect(styleAt(`${E}[?31mx`, 0)).toBeNull()
    expect(styleAt(`${E}[>4;2mx`, 0)).toBeNull()
    expect(styleAt(`${E}[31 mx`, 0)).toBeNull()
  })

  it('reads the C1 CSI form of SGR', () => {
    expect(styleAt('\u009b32mx', 0)?.fg).toBe(2)
  })
})

describe('parseAnsi: hostile input', () => {
  it('survives unterminated sequences', () => {
    expect(parseAnsi(`ok${E}[31`).text).toBe('ok31')
    expect(parseAnsi(`ok${E}]0;title never ends`).text).toBe('ok')
    expect(parseAnsi(`ok${E}P never ends`).text).toBe('ok')
    expect(parseAnsi(`ok${E}`).text).toBe('ok')
    expect(parseAnsi(`ok${E}[`).text).toBe('ok')
  })

  it('removes but does not interpret a huge parameter list', () => {
    const params = '1;'.repeat(200_000) + '31'
    const t0 = performance.now()
    const { text, runs } = parseAnsi(`${E}[${params}mafter`)
    expect(performance.now() - t0).toBeLessThan(500)
    expect(text).toBe('after')
    expect(runs).toEqual([])
    // Up to the cap it is read.
    const ok = '0;'.repeat((SGR_MAX_PARAMS_LENGTH - 2) / 2) + '31'
    expect(ok.length).toBeLessThanOrEqual(SGR_MAX_PARAMS_LENGTH)
    expect(styleAt(`${E}[${ok}mx`, 0)?.fg).toBe(1)
  })

  it('stays linear on many unterminated introducers', () => {
    const line = `${E}[${'9'.repeat(1000)}`.repeat(200)
    const t0 = performance.now()
    parseAnsi(line)
    expect(performance.now() - t0).toBeLessThan(500)
  })

  it('handles huge numbers without overflow surprises', () => {
    expect(styleAt(`${E}[99999999999999999999mx`, 0)).toBeNull()
    expect(styleAt(`${E}[38;5;99999999999999999mx`, 0)).toBeNull()
  })
})

describe('parseAnsi: performance', () => {
  it('parses 10,000 coloured lines quickly', () => {
    const lines = Array.from(
      { length: 10_000 },
      (_, i) => `2026-10-23T12:00:00Z ${E}[32mINFO${E}[0m request ${String(i)} ${E}[38;5;244mtook${E}[0m ${E}[1;31m${String(i % 97)}ms${E}[0m`,
    )
    const t0 = performance.now()
    for (const l of lines) parseAnsi(l)
    const ms = performance.now() - t0
    console.info(`parseAnsi: 10k coloured lines in ${ms.toFixed(1)} ms`)
    expect(ms).toBeLessThan(250)
  })
})

describe('ansiCss', () => {
  const base: AnsiStyle = { fg: null, bg: null, bold: false, dim: false, italic: false, underline: false, inverse: false, strike: false }

  it('maps named colours to theme tokens', () => {
    expect(ansiColorCss(1)).toBe('var(--ansi-red)')
    expect(ansiColorCss(15)).toBe('var(--ansi-bright-white)')
  })

  it('clamps RGB lightness into the theme band', () => {
    expect(ansiColorCss(RGB_BASE + 0x000000)).toContain('clamp(var(--ansi-l-min')
    expect(ansiColorCss(RGB_BASE + 0x0a141e)).toContain('rgb(10 20 30)')
  })

  it('tints backgrounds, and inverts to solid', () => {
    expect(ansiCss({ ...base, bg: 1 }).backgroundColor).toContain('color-mix')
    expect(ansiCss({ ...base, inverse: true })).toEqual({ color: 'var(--background)', backgroundColor: 'var(--foreground)' })
    expect(ansiCss({ ...base, fg: 2, inverse: true })).toEqual({ color: 'var(--background)', backgroundColor: 'var(--ansi-green)' })
  })

  it('maps attributes', () => {
    expect(ansiCss({ ...base, bold: true, dim: true, italic: true, underline: true, strike: true })).toEqual({
      fontWeight: 700,
      opacity: 0.7,
      fontStyle: 'italic',
      textDecorationLine: 'underline line-through',
    })
  })
})
