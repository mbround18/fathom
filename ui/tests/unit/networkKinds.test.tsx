import { render, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'

// `@/store` first: avoids the store <-> descriptor import cycle (see actions.test.ts).
import '@/store'
import { endpoints } from '@/resources/endpoints'
import { ingresses } from '@/resources/ingress'
import { ingressClasses, isDefaultClass } from '@/resources/ingressclasses'
import { networkPolicies, ruleText } from '@/resources/networkpolicies'
import { services } from '@/resources/services'

import {
  ENDPOINTS,
  ENDPOINT_DETAILS,
  INGRESSES,
  INGRESS_DETAILS,
  NETPOLS,
  NETPOL_DETAILS,
} from '../fixtures/network'

interface Sectioned {
  detailSections: { id: string; render: (c: never) => unknown }[]
}

function listItems(d: Sectioned, row: unknown, detail: unknown): ReactNode[] {
  const block = d.detailSections
    .find((s) => s.id === 'spec')
    ?.render({ row, detail, related: undefined } as never) as { items: ReactNode[] }
  return block.items
}

describe('network kinds', () => {
  it('cluster-scoped IngressClass sends SelectIngressClass without namespace', () => {
    expect(ingressClasses.namespaced).toBe(false)
    const cmds = ingressClasses.read?.({
      section: 'network/ingressclasses',
      kind: 'IngressClass',
      namespace: null,
      name: 'nginx',
    })
    expect(cmds?.[0]).toEqual({ type: 'SelectIngressClass', data: { name: 'nginx' } })
  })

  it('delete is a confirmed destructive action for Service, Ingress, NetworkPolicy', () => {
    for (const d of [services, ingresses, networkPolicies]) {
      const a = d.actions.find((x) => x.id === 'delete')
      expect(a?.prompt).toBe('confirm')
      expect(a?.destructive).toBe(true)
    }
    expect(endpoints.actions).toHaveLength(0)
    expect(ingressClasses.actions).toHaveLength(0)
  })

  it('delete plans emit the Delete commands', () => {
    const ing = ingresses.actions[0]?.plan([INGRESSES[0] as never], {})
    expect(ing?.[0]?.cmd).toEqual({
      type: 'DeleteIngress',
      data: { namespace: 'default', name: 'web' },
    })
    const np = networkPolicies.actions[0]?.plan(NETPOLS, {})
    expect(np?.[0]?.cmd).toEqual({
      type: 'DeleteNetworkPolicy',
      data: { namespace: 'default', name: 'deny-all' },
    })
    const sv = services.actions[0]?.plan([{ namespace: 'default', name: 's' } as never], {})
    expect(sv?.[0]?.cmd).toEqual({
      type: 'DeleteService',
      data: { namespace: 'default', name: 's' },
    })
  })

  it('ingress status reflects the load balancer', () => {
    const col = ingresses.columns.find((c) => c.id === 'status')
    expect(col?.text(INGRESSES[0] as never)).toBe('Ready')
    expect(col?.text(INGRESSES[1] as never)).toBe('Pending')
  })

  it('ingress inspector shows rules and TLS', () => {
    render(<div>{listItems(ingresses, INGRESSES[0], INGRESS_DETAILS)}</div>)
    const rule = screen.getByTestId('ingress-rule').textContent
    expect(rule).toContain('app.example.com/api')
    expect(rule).toContain('api:8080')
    expect(screen.getByTestId('ingress-tls').textContent).toContain('app.example.com')
  })

  it('network policy inspector shows selector and rules', () => {
    render(<div>{listItems(networkPolicies, NETPOLS[0], NETPOL_DETAILS)}</div>)
    expect(screen.getByTestId('netpol-selector').textContent).toContain('app=web')
    expect(screen.getByTestId('netpol-ingress-rule').textContent).toContain('role=fe')
    expect(screen.getByTestId('netpol-egress-rule').textContent).toContain(
      '10.0.0.0/8 except 10.1.0.0/16',
    )
    const rule = NETPOL_DETAILS.ingress_rules[0]
    expect(rule && ruleText(rule, 'from')).toContain('80/TCP')
  })

  it('endpoints inspector lists ready and not-ready addresses', () => {
    render(<div>{listItems(endpoints, ENDPOINTS[0], ENDPOINT_DETAILS)}</div>)
    const t = screen.getByTestId('endpoint-subset').textContent
    expect(t).toContain('not ready 10.1.0.2')
    expect(t).toContain('ready 10.1.0.1 → Pod/default/web-1')
  })

  it('ingress class default flag comes from the annotation', () => {
    const base = {
      name: 'nginx',
      creation_timestamp: '',
      labels: {},
      controller: 'x',
      parameters: null,
      raw_json: '',
    }
    const key = 'ingressclass.kubernetes.io/is-default-class'
    expect(isDefaultClass({ ...base, annotations: { [key]: 'true' } })).toBe(true)
    expect(isDefaultClass({ ...base, annotations: {} })).toBe(false)
  })
})
