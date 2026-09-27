import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { PrometheusStatus } from '@/features/metrics/PrometheusStatus'
import {
  prometheusStatusWord,
  splitPrometheusStatus,
} from '@/features/metrics/prometheusStatusText'

function el(root: Element | null, selector: string): Element {
  const found = root?.querySelector(selector)
  if (!found) throw new Error(`missing ${selector}`)
  return found
}

describe('PrometheusStatus (feedback #32)', () => {
  it('shows a short summary and keeps the raw response in collapsed details', () => {
    render(
      <PrometheusStatus
        test={{
          success: false,
          kind: 'QueryFailed',
          message:
            'Found Prometheus at monitoring/prometheus:9090, but the query failed: 404 page not found',
          detail:
            'GET /api/v1/namespaces/monitoring/services/prometheus:9090/proxy/api/v1/query?query=up\n\n404 page not found\n',
        }}
      />,
    )
    const status = screen.getByTestId('prometheus-test-result')
    expect(status.getAttribute('data-kind')).toBe('QueryFailed')
    expect(status.textContent).toContain('Query failed:')
    expect(status.textContent).toContain('Found Prometheus at monitoring/prometheus:9090')
    const details = screen.getByTestId('prometheus-test-details')
    expect(details.tagName).toBe('DETAILS')
    expect(details.hasAttribute('open')).toBe(false)
    expect(el(details, 'pre').textContent).toContain('GET /api/v1/namespaces/')
  })

  it('never renders a raw HTML body inline, even from an older backend', () => {
    const html =
      '<html><head><title>503</title></head><body><h1>Service Unavailable</h1>\n<p>' +
      'x'.repeat(400) +
      '</p></body></html>'
    render(<PrometheusStatus test={{ success: false, message: html }} />)
    const summary = el(screen.getByTestId('prometheus-test-result'), 'p').textContent
    expect(summary).not.toContain('<')
    expect(summary.length).toBeLessThan(200)
    expect(summary).toContain('Failed:')
    // The full raw body is still available, only inside the collapsed details.
    const pre = el(screen.getByTestId('prometheus-test-details'), 'pre')
    expect(pre.textContent).toContain('<h1>Service Unavailable</h1>')
    expect(pre.textContent).toContain('x'.repeat(400))
  })

  it('multi-line messages keep only the first line inline', () => {
    const { summary, detail } = splitPrometheusStatus({
      success: false,
      message: 'failed\n404 page not found\nservice 503',
    })
    expect(summary).toBe('failed')
    expect(detail).toContain('service 503')
  })

  it('a short message with no detail renders without a details toggle', () => {
    render(
      <PrometheusStatus
        test={{ success: false, kind: 'NotFound', message: 'No Prometheus found in this cluster.' }}
      />,
    )
    expect(screen.getByTestId('prometheus-test-result').textContent).toBe(
      'Not found: No Prometheus found in this cluster.',
    )
    expect(screen.queryByTestId('prometheus-test-details')).toBeNull()
  })

  it('distinguishes not found, unreachable and query errors in the status word', () => {
    expect(prometheusStatusWord(undefined)).toBe('not tested')
    expect(prometheusStatusWord({ success: true, message: 'ok' })).toBe('connected')
    expect(prometheusStatusWord({ success: false, message: '', kind: 'NotFound' })).toBe(
      'not found',
    )
    expect(prometheusStatusWord({ success: false, message: '', kind: 'Unreachable' })).toBe(
      'found but not reachable',
    )
    expect(prometheusStatusWord({ success: false, message: '', kind: 'QueryFailed' })).toBe(
      'found but queries fail',
    )
    expect(prometheusStatusWord({ success: false, message: '' })).toBe('not connected')
  })
})
