/** NetworkPolicies (034 batch B): pod selector, policy types, ingress/egress rules, delete. */
import { StatusBadge } from '@/components/StatusBadge'
import { selectCmd } from '@/protocol/commands'
import type {
  NetworkPolicyDetails,
  NetworkPolicySummary,
  PolicyRule,
  RuleSelector,
} from '@/protocol/types'

import { deleteAction } from './actions'
import { compareNumber, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'network/networkpolicies'

export function selectorText(s: RuleSelector): string {
  if (s.selector_type === 'IPBlock') {
    return `${s.ip_block ?? ''}${s.except.length ? ` except ${s.except.join(', ')}` : ''}`
  }
  const labels = Object.entries(s.labels ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join(',')
  return `${s.selector_type}${labels ? ` ${labels}` : ' (all)'}`
}

export function ruleText(r: PolicyRule, dir: 'from' | 'to'): string {
  const peers = (dir === 'from' ? r.from_selectors : r.to_selectors).map(selectorText)
  const ports = r.ports.length
    ? `${r.ports.map(String).join(',')}${r.protocols.length ? `/${r.protocols.join(',')}` : ''}`
    : 'all ports'
  return `${dir} ${peers.join('; ') || 'anywhere'} on ${ports}`
}

const selectorLabel = (m: Record<string, string>) =>
  Object.entries(m)
    .map(([k, v]) => `${k}=${v}`)
    .join(', ') || 'all pods'

function ruleItems(rules: PolicyRule[], dir: 'from' | 'to', id: string) {
  return rules.map((r, i) => (
    <div key={`${id}-${String(i)}`} className="font-mono" data-testid={`netpol-${id}-rule`}>
      {id}: {ruleText(r, dir)}
    </div>
  ))
}

const sections: DetailSection<NetworkPolicySummary, NetworkPolicyDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
      ]
      if (detail) fields.push({ label: 'Created', value: detail.creation_timestamp, mono: true })
      return { type: 'fields', fields }
    },
  },
  {
    id: 'status',
    title: 'Status',
    render: ({ row, detail }) =>
      row
        ? {
            type: 'fields',
            fields: [
              { label: 'State', value: <StatusBadge status="Active" /> },
              { label: 'Policy types', value: detail?.policy_types.join(', ') || '—', mono: true },
              { label: 'Ingress rules', value: String(row.ingress_rules_count), mono: true },
              { label: 'Egress rules', value: String(row.egress_rules_count), mono: true },
            ],
          }
        : null,
  },
  {
    id: 'spec',
    title: 'Spec',
    render: ({ detail }) =>
      detail
        ? {
            type: 'list',
            items: [
              <div key="sel" className="font-mono" data-testid="netpol-selector">
                Pod selector: {selectorLabel(detail.pod_selector)}
              </div>,
              ...ruleItems(detail.ingress_rules, 'from', 'ingress'),
              ...ruleItems(detail.egress_rules, 'to', 'egress'),
            ],
          }
        : null,
  },
  {
    id: 'labels',
    title: 'Labels',
    render: ({ detail }) => (detail ? { type: 'map', entries: detail.labels } : null),
  },
  {
    id: 'annotations',
    title: 'Annotations',
    render: ({ detail }) => (detail ? { type: 'map', entries: detail.annotations } : null),
  },
]

const rulesText = (p: NetworkPolicySummary) =>
  `${String(p.ingress_rules_count)} in / ${String(p.egress_rules_count)} out`

export const networkPolicies: ResourceDescriptor<NetworkPolicySummary, NetworkPolicyDetails> = {
  section: SECTION,
  kind: 'NetworkPolicy',
  label: 'Network policies',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 150,
      grow: 3,
      cell: (p) => p.name,
      text: (p) => p.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'status',
      header: 'Rules',
      priority: 1,
      minWidth: 120,
      status: true,
      cell: rulesText,
      text: rulesText,
      sort: (a, b) =>
        compareNumber(
          a.ingress_rules_count + a.egress_rules_count,
          b.ingress_rules_count + b.egress_rules_count,
        ),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (p) => p.namespace,
      text: (p) => p.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'selector',
      header: 'Pod selector',
      priority: 3,
      minWidth: 140,
      grow: 2,
      cell: (p) => p.pod_selector,
      text: (p) => p.pod_selector,
      sort: (a, b) => compareText(a.pod_selector, b.pod_selector),
    },
  ],
  rowKey: (p) => `${p.namespace}/${p.name}`,
  ref: (p) => ({ section: SECTION, kind: 'NetworkPolicy', namespace: p.namespace, name: p.name }),
  refresh: () => ({ type: 'RefreshNetworkPolicies' }),
  read: (ref) => {
    const cmd = selectCmd('NetworkPolicy', ref.namespace ?? '', ref.name)
    return [...(cmd ? [cmd] : []), { type: 'RefreshEvents' }]
  },
  detailSections: sections,
  actions: [
    deleteAction<NetworkPolicySummary>((namespace, name) => ({
      type: 'DeleteNetworkPolicy',
      data: { namespace, name },
    })),
  ],
  selectListEvent: (e) => (e.type === 'NetworkPoliciesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'NetworkPolicyDetailsUpdated' ? (d.data as NetworkPolicyDetails) : undefined,
  rawJson: (d) => d.raw_json,
}
