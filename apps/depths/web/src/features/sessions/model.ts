/**
 * Formatting for Your sessions and the admin panel (spec 052 US2, US3). Everything shown is plain
 * text: React escapes it, and these keep client-supplied strings short and free of control
 * characters (FR-014, acceptance US2-4).
 */
import type { NetworkView, SessionView } from '@/protocol/generated'

export const BROWSER_MAX = 48

/** A client-supplied browser string, single-line and at most `max` characters. */
export function shortBrowser(s: string, max = BROWSER_MAX): string {
  // eslint-disable-next-line no-control-regex -- stripping control characters is the point
  const clean = s.replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').replace(/\s+/g, ' ').trim()
  if (clean === '') return 'Unknown browser'
  return clean.length > max ? `${clean.slice(0, max - 1)}…` : clean
}

/** The current one first, then the most recently used. */
export function sortSessions(list: readonly SessionView[]): SessionView[] {
  return [...list].sort((a, b) => Number(b.current) - Number(a.current) || b.last_used.localeCompare(a.last_used))
}

/** The current one first, then the most recently used. */
export function sortNetworks(list: readonly NetworkView[]): NetworkView[] {
  return [...list].sort((a, b) => Number(b.current) - Number(a.current) || b.last_used.localeCompare(a.last_used))
}

const V4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/
const V6 = /^[0-9a-f:]+$/i

/** Why a whitelist entry would be refused before it is sent, or `null`. The server checks again. */
export function whitelistProblem(cidr: string): string | null {
  const v = cidr.trim()
  if (v === '') return 'Enter an address or a block, like 198.51.100.7 or 198.51.100.0/24.'
  const [addr = '', bits, ...extra] = v.split('/')
  const v4 = V4.test(addr)
  const v6 = !v4 && V6.test(addr) && addr.includes(':')
  if (extra.length > 0 || (!v4 && !v6)) return 'That is not an address or a block, like 198.51.100.0/24.'
  if (bits !== undefined) {
    const n = /^\d{1,3}$/.test(bits) ? Number(bits) : NaN
    if (!(n >= 0 && n <= (v4 ? 32 : 128))) return `The block size must be 0 to ${v4 ? '32' : '128'}.`
  }
  return null
}
