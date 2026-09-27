import type { PrometheusStatusKind } from '@/protocol/types'

/** The backend's `PrometheusConnectionTest` payload. */
export interface PrometheusTest {
  success: boolean
  message: string
  kind?: PrometheusStatusKind
  detail?: string
}

export const PROMETHEUS_STATUS_LABEL: Record<PrometheusStatusKind, string> = {
  Connected: 'Connected',
  NotFound: 'Not found',
  Unreachable: 'Not reachable',
  QueryFailed: 'Query failed',
}

/** Short wording for the Metrics pane's one-line "Prometheus: …" status. */
export function prometheusStatusWord(test: PrometheusTest | undefined): string {
  if (!test) return 'not tested'
  if (test.success) return 'connected'
  switch (test.kind) {
    case 'NotFound':
      return 'not found'
    case 'Unreachable':
      return 'found but not reachable'
    case 'QueryFailed':
      return 'found but queries fail'
    default:
      return 'not connected'
  }
}

const SUMMARY_MAX = 160

/**
 * Split a status into a one-line summary and the rest for "Details" (feedback #32). The backend
 * already sends a short `message` and the raw response in `detail`; this also guards against a
 * long, multi-line or HTML message (an older backend, or an unexpected error path) so a raw
 * response body is never rendered inline.
 */
export function splitPrometheusStatus(test: PrometheusTest): { summary: string; detail?: string } {
  const message = test.message
  const firstLine = message.split(/\r?\n/, 1)[0] ?? ''
  const stripped = firstLine
    .replace(/<[^>]*>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
  const clipped =
    stripped.length > SUMMARY_MAX ? `${stripped.slice(0, SUMMARY_MAX).trimEnd()}…` : stripped
  const messageIsRaw = clipped !== message.trim()
  const parts = [test.detail, messageIsRaw ? message : undefined].filter(
    (p): p is string => typeof p === 'string' && p.trim() !== '',
  )
  return {
    summary: clipped || (test.success ? 'Connected' : 'Prometheus request failed'),
    detail: parts.length > 0 ? parts.join('\n\n') : undefined,
  }
}
