/**
 * Join tokens and join requests (spec 055 T148, contracts/join.md): the pure parts the Hosts page's
 * Join requests tab and Settings → Hosts share.
 *
 * A join token is a compact JWS. Keel never shows it again after issue or rotate, so the claims are
 * decoded in the page from the one copy it holds, to show what a host will be told. Decoding does
 * not verify anything: the runner cannot either, and the server checks the signature.
 */
import type { JoinRequestInfo, JoinRequestState } from '@/protocol/wire'

/** What a join token tells a host, decoded without verifying. */
export interface JoinClaims {
  issuer: string
  workspace: string
  jti: string
  issuedAt: number
  expiresAt: number
  server: string
  hostCa: string
  origin: string
  rpId: string
  inventoryName: string | null
  profiles: string[]
}

function base64url(part: string): string | null {
  if (!/^[A-Za-z0-9_-]*$/.test(part)) return null
  const b64 = part.replace(/-/g, '+').replace(/_/g, '/')
  try {
    const bin = atob(b64 + '='.repeat((4 - (b64.length % 4)) % 4))
    return new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)))
  } catch {
    return null
  }
}

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null)
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null)

/** The claims of `token`, or `null` when it does not read as a join token. */
export function decodeJoinClaims(token: string): JoinClaims | null {
  const parts = token.trim().split('.')
  if (parts.length !== 3) return null
  const json = base64url(parts[1] ?? '')
  if (json === null) return null
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return null
  }
  if (typeof raw !== 'object' || raw === null) return null
  const c = raw as Record<string, unknown>
  const issuer = str(c.iss)
  const workspace = str(c.sub)
  const jti = str(c.jti)
  const issuedAt = num(c.iat)
  const expiresAt = num(c.exp)
  const server = str(c['keel.server'])
  const hostCa = str(c['keel.host_ca'])
  const origin = str(c['keel.origin'])
  const rpId = str(c['keel.rp_id'])
  if (
    c.aud !== 'keel-runner' ||
    issuer === null ||
    workspace === null ||
    jti === null ||
    issuedAt === null ||
    expiresAt === null ||
    server === null ||
    hostCa === null ||
    origin === null ||
    rpId === null
  ) {
    return null
  }
  const profiles = Array.isArray(c['keel.profiles']) ? c['keel.profiles'].filter((p): p is string => typeof p === 'string') : []
  return {
    issuer,
    workspace,
    jti,
    issuedAt,
    expiresAt,
    server,
    hostCa,
    origin,
    rpId,
    inventoryName: str(c['keel.inventory_name']),
    profiles,
  }
}

/** Seconds since the epoch to RFC 3339 UTC, without milliseconds. */
export function epochToRfc3339(seconds: number): string {
  return new Date(seconds * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z')
}

/** The accept form's starting values: the host's suggested name, and the token's profiles. */
export function acceptPrefill(req: JoinRequestInfo): { inventoryName: string; profiles: string[] } {
  const profiles = req.suggested_profiles.length > 0 ? req.suggested_profiles : req.allowed_profiles.filter((p) => p === 'base')
  return { inventoryName: req.inventory_name, profiles: [...profiles] }
}

/** Profiles the host's own policy does not allow: Keel refuses an accept that names any. */
export function outsideAllowed(profiles: readonly string[], allowed: readonly string[]): string[] {
  return profiles.filter((p) => !allowed.includes(p))
}

/** `items` with `req` in place (or added, newest seen first), keeping only `state` when set. */
export function upsertRequest(items: JoinRequestInfo[], req: JoinRequestInfo, state: JoinRequestState | null): JoinRequestInfo[] {
  const rest = items.filter((r) => r.id !== req.id)
  if (state !== null && req.state !== state) return rest
  return [req, ...rest].sort((a, b) => b.last_seen_at.localeCompare(a.last_seen_at))
}

/** Why a request is no longer pending, in words. Unknown reasons are shown as sent. */
export const REASON_TEXT: Record<string, string> = {
  rejected: 'Rejected by an operator',
  token_rotated: 'The join token was rotated or disabled',
  expired: 'Waited 7 days without a decision',
}

export function reasonText(reason: string | null): string | null {
  if (reason === null) return null
  return REASON_TEXT[reason] ?? reason
}

/** Expiry choices for issue and rotate, in days (the server takes 1 to 365). */
export const EXPIRY_DAYS = [1, 7, 30, 90, 365] as const
export const DEFAULT_EXPIRY_DAYS = 30
