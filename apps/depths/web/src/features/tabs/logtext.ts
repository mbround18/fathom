/**
 * Log lines are untrusted text from containers (FR-026). The ring keeps them raw; the shared log
 * view (`@companion/terminal`) parses only SGR colour and renders text, never markup, so a line
 * cannot move the cursor, retitle the window, emit a hyperlink or hide text from the operator.
 * `stripAnsi` is the plain text, with every control sequence and control character removed.
 */
export { stripAnsi } from '@companion/terminal'

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
