/**
 * Log lines are untrusted text from containers (FR-026). They are rendered as text, never as
 * markup, and terminal control sequences are removed first: a log line must not be able to move
 * the cursor, retitle the window, emit a hyperlink or hide text from the operator.
 */

// CSI (`ESC [ … final`), OSC (`ESC ] … BEL|ST`), DCS/SOS/PM/APC strings, then any other
// two-byte escape, then the C1 CSI introducer. Built from code points so no literal control
// character sits in the source.
const ESC = String.fromCharCode(0x1b)
const BEL = String.fromCharCode(0x07)
const CSI = String.fromCharCode(0x9b)
const SEQUENCES = new RegExp(
  [
    `${ESC}\\[[0-?]*[ -/]*[@-~]`,
    `${ESC}\\][^${BEL}${ESC}]*(?:${BEL}|${ESC}\\\\)?`,
    `${ESC}[PX^_][^${ESC}]*(?:${ESC}\\\\)?`,
    `${ESC}[@-_]`,
    `${CSI}[0-?]*[ -/]*[@-~]`,
  ].join('|'),
  'g',
)

/** C0 and C1 controls other than tab, plus DEL and a stray ESC. */
function isControl(code: number): boolean {
  return (code < 0x20 && code !== 0x09) || (code >= 0x7f && code < 0xa0)
}

/** The line with every terminal control sequence and control character removed. */
export function stripAnsi(line: string): string {
  const noSeq = line.replace(SEQUENCES, '')
  let out = ''
  for (const ch of noSeq) {
    const code = ch.codePointAt(0) ?? 0
    // Bidi overrides and isolates could reorder what the operator reads.
    if (isControl(code) || (code >= 0x202a && code <= 0x202e) || (code >= 0x2066 && code <= 0x2069)) continue
    out += ch
  }
  return out
}

/**
 * The time a line was written. `docker logs --timestamps` puts an RFC 3339 time first;
 * `docker compose logs --timestamps` puts it after the `service-1  | ` prefix.
 */
const STAMP = /^(?:[^|]{0,128}\|\s)?(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))/

export function lineTime(line: string): { ms: number; stamp: string } | null {
  const m = STAMP.exec(line)
  const stamp = m?.[1]
  if (!stamp) return null
  const ms = Date.parse(stamp)
  return Number.isNaN(ms) ? null : { ms, stamp }
}

/** `datetime-local` value (local wall time) to epoch ms; `null` when empty or invalid. */
export function localInputMs(value: string): number | null {
  if (value === '') return null
  const ms = new Date(value).getTime()
  return Number.isNaN(ms) ? null : ms
}
