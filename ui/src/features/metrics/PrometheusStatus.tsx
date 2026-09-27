import {
  PROMETHEUS_STATUS_LABEL,
  type PrometheusTest,
  splitPrometheusStatus,
} from './prometheusStatusText'

/** One short status line, with any raw response behind a collapsed "Details" (feedback #32). */
export function PrometheusStatus({ test }: { test: PrometheusTest }) {
  const { summary, detail } = splitPrometheusStatus(test)
  const label = test.success
    ? 'Connected'
    : test.kind
      ? PROMETHEUS_STATUS_LABEL[test.kind]
      : 'Failed'
  return (
    <div
      role="status"
      data-testid="prometheus-test-result"
      data-kind={test.kind ?? (test.success ? 'Connected' : 'Failed')}
      className={test.success ? 'text-xs' : 'text-xs text-destructive'}
    >
      <p className="break-words">
        <span className="font-medium">{label}: </span>
        {summary}
      </p>
      {detail && (
        <details className="mt-1 text-muted-foreground" data-testid="prometheus-test-details">
          <summary className="cursor-pointer select-none">Details</summary>
          <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 font-mono text-[11px] break-all whitespace-pre-wrap">
            {detail}
          </pre>
        </details>
      )}
    </div>
  )
}
