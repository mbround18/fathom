/**
 * Container containment for display (051): which chip a container or stack shows, whether its shell
 * button is offered, and when an admin is told the daemon runs as root. Pure; the server re-inspects
 * at shell open and is the one that decides (051 FR-008), so a missing rating never disables
 * anything here.
 */
import type { ComposeWarning, ContainerSecurity, Features, Role, SecurityLevel } from '@/protocol/generated'

/** The guide the chips and the rootful notice link to. */
export const SECURITY_GUIDE = 'https://github.com/mbround18/fathom/blob/main/docs/guides/depths/security.md'

export interface Chip {
  level: Exclude<SecurityLevel, 'ok'>
  label: string
  /** Theme classes: amber for root, red for escape. */
  tone: string
}

const CHIPS: Record<Exclude<SecurityLevel, 'ok'>, Chip> = {
  root: { level: 'root', label: 'root', tone: 'border-status-warn text-status-warn bg-status-warn/10' },
  escape: { level: 'escape', label: 'escape', tone: 'border-status-err text-status-err bg-status-err/10' },
}

/** The chip for a level; none for `ok` or an unrated container. */
export function chipFor(level: SecurityLevel | null | undefined): Chip | null {
  if (level === undefined || level === null || level === 'ok') return null
  return CHIPS[level]
}

/** The reasons, one sentence each, worst first, as the chip's tooltip says them. */
export function reasonLines(security: ContainerSecurity | undefined): string[] {
  return security?.reasons.filter((r) => r.severity !== 'info').map((r) => r.message) ?? []
}

export interface ShellGate {
  enabled: boolean
  /** Why it is off, for the button's label and tooltip. */
  reason: string | null
}

/**
 * Whether the shell button is offered for a container, past the feature flag: off when the
 * server's rule refuses it, or keeps it for admins and this person is not one.
 */
export function shellGate(security: ContainerSecurity | undefined, role: Role | undefined): ShellGate {
  const rule = security?.shell
  if (rule === 'refused') {
    return { enabled: false, reason: 'Shells are refused: this container can reach the host.' }
  }
  if (rule === 'admin_only' && role !== 'admin') {
    return { enabled: false, reason: 'Only an administrator may open a shell: this container runs as root on the host.' }
  }
  return { enabled: true, reason: null }
}

/** An admin is told when the daemon runs as root (051 FR-006); nobody else learns its mode. */
export function rootfulNotice(role: Role | undefined, features: Features | undefined): boolean {
  return role === 'admin' && features?.daemon === 'rootful'
}

/** The worst level among compose findings; `ok` when none weakens containment. */
export function worstFinding(warnings: readonly ComposeWarning[] | undefined): SecurityLevel {
  const all = warnings ?? []
  if (all.some((w) => w.severity === 'escape')) return 'escape'
  if (all.some((w) => w.severity === 'root')) return 'root'
  return 'ok'
}
