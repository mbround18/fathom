/** 036-incluster-daemon: labels, tones and selectors shared by the daemon UI. */
import type { DaemonState } from '@/protocol/types'
import { useStore } from '@/store'
import type { DaemonContextState } from '@/store/handlers/daemon'

/** Copy-only: Fathom never installs anything into a cluster for the user. */
export const INSTALL_COMMAND = 'kubectl apply -k k8s/overlays/daemon'

/** 043 US4: the preference switch, named for what it controls (feedback #12). */
export const DAEMON_SWITCH_LABEL = "Use Fathom's in-cluster daemon"

/** 043 US4 scenario 1, verbatim: what on and off do. */
export const DAEMON_SWITCH_DESCRIPTION =
  'On: Fathom checks this cluster for its optional, read-only daemon and, if found, uses it for cluster history and a faster topology. Off: Fathom never contacts it. Nothing is installed either way.'

/** 043 US4: the Cluster history panel's first line (formerly "While you were away"). */
export const CLUSTER_HISTORY_SUBTITLE =
  'Changes the in-cluster daemon recorded, including while Fathom was closed.'

export const STATE_LABEL: Record<DaemonState, string> = {
  off: 'Off',
  notInstalled: 'Not installed',
  probing: 'Probing',
  available: 'Connected',
  unavailable: 'Unavailable',
  noAccess: 'No access',
  versionMismatch: 'Version mismatch',
}

export type DaemonTone = 'ok' | 'warn' | 'err' | 'muted'

export const STATE_TONE: Record<DaemonState, DaemonTone> = {
  off: 'muted',
  notInstalled: 'muted',
  probing: 'warn',
  available: 'ok',
  unavailable: 'warn',
  noAccess: 'err',
  versionMismatch: 'err',
}

/** Glyph + word, never colour alone. */
export const STATE_GLYPH: Record<DaemonState, string> = {
  off: '○',
  notInstalled: '○',
  probing: '◌',
  available: '●',
  unavailable: '▲',
  noAccess: '✕',
  versionMismatch: '✕',
}

/** One-line explanation per state (the verbatim `detail`, when any, is shown beside it). */
export const STATE_HELP: Record<DaemonState, string> = {
  off: 'Fathom is not contacting an in-cluster daemon for this context.',
  notInstalled: 'No Fathom daemon is installed in this cluster. Everything works without it.',
  probing: 'Checking whether a daemon is installed.',
  available: 'Controller events and topology are streamed from the in-cluster daemon.',
  unavailable: 'The daemon did not respond. Fathom retries with backoff.',
  noAccess: 'Your credentials are not allowed to use the daemon.',
  versionMismatch: 'The daemon speaks an incompatible protocol version.',
}

export const EMPTY_DAEMON: DaemonContextState = {
  status: null,
  session: 0,
  timeline: [],
  truncated: null,
  topology: { seq: 0, nodes: {}, edges: [] },
  metrics: {},
}

/** The active context's daemon state (never null). */
export function useActiveDaemon(): { context: string | undefined; daemon: DaemonContextState } {
  const context = useStore((s) => s.session.activeContext)
  const daemon = useStore((s) => (context ? s.daemon[context] : undefined)) ?? EMPTY_DAEMON
  return { context, daemon }
}

export function daemonState(d: DaemonContextState): DaemonState {
  return d.status?.state ?? 'off'
}

export function formatTime(iso: string): string {
  const t = new Date(iso)
  if (Number.isNaN(t.getTime())) return iso
  return t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}
