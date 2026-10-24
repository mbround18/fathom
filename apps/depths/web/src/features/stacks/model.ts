/**
 * Pure helpers for the stack views: filtering, labels and masking. No React, no store, so the
 * unit tests cover them directly and the list stays cheap to filter at 200 stacks (SC-004).
 */
import type {
  Container,
  ForeignProject,
  PublishedPort,
  RuntimeContainerState,
  Stack,
  StackList,
  StackStatus,
} from '@/protocol/generated'

export type StatusFilter = StackStatus | 'all'

export const STATUS_ORDER: readonly StackStatus[] = [
  'running',
  'partial',
  'stopped',
  'not_deployed',
  'invalid',
]

export const STATUS_LABEL: Record<StackStatus, string> = {
  running: 'Running',
  partial: 'Partly running',
  stopped: 'Stopped',
  not_deployed: 'Not deployed',
  invalid: 'Invalid',
}

/** Case-insensitive substring match on the name, then the status filter. */
export function filterStacks(stacks: readonly Stack[], query: string, status: StatusFilter): Stack[] {
  const q = query.trim().toLowerCase()
  return stacks.filter(
    (s) => (status === 'all' || s.status === status) && (q === '' || s.name.toLowerCase().includes(q)),
  )
}

/** Foreign projects and standalone containers have no stack status: a status filter hides them. */
export function filterByName<T extends { name: string }>(items: readonly T[], query: string, status: StatusFilter): T[] {
  if (status !== 'all') return []
  const q = query.trim().toLowerCase()
  return q === '' ? [...items] : items.filter((i) => i.name.toLowerCase().includes(q))
}

/**
 * Why a stack cannot be acted on, or `null` when it can.
 *
 * The wire `Stack` carries no parser message for an unparseable compose file (only `read_error`
 * for I/O), so that case falls back to a generic reason; the server's own text, when it sends one,
 * arrives in `StackList.warnings` and is shown verbatim above the list.
 */
export function blockedReason(s: Stack): string | null {
  if (s.read_error !== null) return s.read_error
  if (!s.valid_name) return 'Not a valid compose project name'
  if (s.status === 'invalid') return 'The compose file does not parse'
  return null
}

/** Foreign projects started by hand (`-p`) from a stack's directory, keyed by that stack. */
export function elsewhereByStack(foreign: readonly ForeignProject[]): Map<string, ForeignProject[]> {
  const out = new Map<string, ForeignProject[]>()
  for (const p of foreign) {
    if (p.elsewhere_of === null) continue
    const list = out.get(p.elsewhere_of) ?? []
    list.push(p)
    out.set(p.elsewhere_of, list)
  }
  return out
}

export function stateLabel(state: RuntimeContainerState): string {
  return typeof state === 'string' ? state : state.unknown
}

export function formatPort(p: PublishedPort): string {
  const inner = `${String(p.container_port)}/${p.protocol}`
  if (p.host_port === null) return inner
  const ip = p.host_ip && p.host_ip !== '0.0.0.0' && p.host_ip !== '::' ? `${p.host_ip}:` : ''
  return `${ip}${String(p.host_port)}→${inner}`
}

/** Distinct published ports (IPv4 and IPv6 bindings of one port collapse to one entry). */
export function formatPorts(ports: readonly PublishedPort[]): string[] {
  return [...new Set(ports.map(formatPort))]
}

/** `YYYY-MM-DD HH:mm` in the viewer's time zone; the raw string when it does not parse. */
export function formatTime(iso: string | null): string {
  if (iso === null) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  const pad = (n: number): string => String(n).padStart(2, '0')
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

const SECRET_KEY = /PASS|SECRET|TOKEN|KEY|PRIVATE|CREDENTIAL/i
export const MASK = '********'

/**
 * Masks the values of secret-looking keys in `.env` text, by the same rule the server uses
 * (contracts/protocol.md `RevealEnv`). Comments, blank lines and non-secret values are kept as is.
 */
export function maskEnv(text: string): string {
  return text
    .split('\n')
    .map((line) => {
      const m = /^(\s*(?:export\s+)?)([^=#\s][^=]*?)(\s*=\s*)(.*)$/.exec(line)
      if (!m) return line
      const [, lead = '', key = '', eq = '', value = ''] = m
      return SECRET_KEY.test(key) && value !== '' ? `${lead}${key}${eq}${MASK}` : line
    })
    .join('\n')
}

export function containersOf(s: Stack): Container[] {
  return s.services.flatMap((v) => v.containers)
}

/** Every stack name the list knows, for detail lookups. */
export function findStack(list: StackList | null, name: string): Stack | undefined {
  return list?.stacks.find((s) => s.name === name)
}

export function stackHref(name: string): string {
  return `/stacks/${encodeURIComponent(name)}`
}

export interface StackNote {
  key: string
  tone: 'err' | 'warn'
  text: string
}

/** Notes shown under a stack, in the list and at the top of its detail view. */
export function stackNotes(stack: Stack, elsewhere: readonly ForeignProject[]): StackNote[] {
  const notes: StackNote[] = []
  const blocked = blockedReason(stack)
  if (stack.read_error !== null) notes.push({ key: 'read', tone: 'err', text: `Cannot read: ${stack.read_error}` })
  else if (blocked !== null) notes.push({ key: 'invalid', tone: 'err', text: blocked })
  if (stack.shadowed_files.length > 0) {
    notes.push({
      key: 'shadowed',
      tone: 'warn',
      text: `Using ${stack.compose_file}; also present and ignored: ${stack.shadowed_files.join(', ')}`,
    })
  }
  for (const p of elsewhere) {
    notes.push({ key: `elsewhere-${p.name}`, tone: 'warn', text: `Running elsewhere as project ${p.name}` })
  }
  if (stack.read_only_reason !== null) notes.push({ key: 'ro', tone: 'warn', text: `Read-only: ${stack.read_only_reason}` })
  if (stack.too_large) notes.push({ key: 'large', tone: 'warn', text: 'A file is too large to load for editing' })
  return notes
}
