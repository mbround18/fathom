/** Batch C config kinds: ConfigMaps, Secrets, ResourceQuotas, LimitRanges, PodDisruptionBudgets. */
import { StatusBadge } from '@/components/StatusBadge'
import type {
  ConfigMapDetails,
  ConfigMapSummary,
  LimitRangeDetails,
  LimitRangeSummary,
  PodDisruptionBudgetDetails,
  PodDisruptionBudgetSummary,
  ResourceQuotaDetails,
  ResourceQuotaSummary,
  SecretDetails,
  SecretSummary,
} from '@/protocol/types'

import {
  gatedDelete,
  labelSections,
  metadataSection,
  nameColumn,
  namespaceColumn,
  parseQuantity,
} from './batchCCommon'
import { SecretKeys } from './secretKeys'
import { compareNumber, compareStatus, compareText } from './search'
import type { ResourceDescriptor } from './types'

const CM = 'config/configmaps'
const SECRETS = 'config/secrets'
const QUOTAS = 'config/resourcequotas'
const LIMITS = 'config/limitranges'
const PDB = 'config/poddisruptionbudgets'

const dash = (v: string | null | undefined) => (v && v.length > 0 ? v : '—')
const num = (n: number | null | undefined) => (n === null || n === undefined ? '—' : String(n))
const ref = (section: string, kind: string) => (r: { namespace: string; name: string }) => ({
  section,
  kind,
  namespace: r.namespace,
  name: r.name,
})
const readNs =
  (
    type:
      | 'SelectConfigMap'
      | 'SelectSecret'
      | 'SelectResourceQuota'
      | 'SelectLimitRange'
      | 'SelectPodDisruptionBudget',
  ) =>
  (r: { namespace: string | null; name: string }) => [
    { type, data: { namespace: r.namespace ?? '', name: r.name } } as const,
    { type: 'RefreshEvents' } as const,
  ]

// ── ConfigMaps ──────────────────────────────────────────────────────────────

export const configMaps: ResourceDescriptor<ConfigMapSummary, ConfigMapDetails> = {
  section: CM,
  kind: 'ConfigMap',
  label: 'ConfigMaps',
  namespaced: true,
  columns: [
    nameColumn(),
    namespaceColumn(),
    {
      id: 'keys',
      header: 'Keys',
      priority: 1,
      minWidth: 60,
      align: 'end',
      cell: (r) => String(r.keys_count),
      text: (r) => String(r.keys_count),
      sort: (a, b) => compareNumber(a.keys_count, b.keys_count),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: ref(CM, 'ConfigMap'),
  refresh: () => ({ type: 'RefreshConfigMaps' }),
  read: readNs('SelectConfigMap'),
  detailSections: [
    metadataSection(true),
    {
      id: 'data',
      title: 'Data',
      render: ({ detail }) =>
        detail
          ? {
              type: 'list',
              items: detail.keys.map((k) => (
                <details key={k} data-testid={`configmap-key-${k}`}>
                  <summary className="cursor-pointer">{k}</summary>
                  <pre className="mt-1 max-h-64 overflow-auto bg-muted/40 p-2 break-all whitespace-pre-wrap">
                    {detail.data[k] ?? ''}
                  </pre>
                </details>
              )),
            }
          : null,
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeleteConfigMap', data: { namespace, name } }),
      'configmaps',
    ),
  ],
  selectListEvent: (e) => (e.type === 'ConfigMapsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ConfigMapDetailsUpdated' ? (d.data as ConfigMapDetails) : undefined,
  rawJson: (d) => d.raw_json,
}

// ── Secrets ─────────────────────────────────────────────────────────────────

export const secrets: ResourceDescriptor<SecretSummary, SecretDetails> = {
  section: SECRETS,
  kind: 'Secret',
  label: 'Secrets',
  namespaced: true,
  columns: [
    nameColumn(),
    {
      id: 'status',
      header: 'Type',
      priority: 1,
      minWidth: 120,
      status: true,
      cell: (r) => <StatusBadge status={r.secret_type} />,
      text: (r) => r.secret_type,
      sort: (a, b) => compareStatus(a.secret_type, b.secret_type),
    },
    namespaceColumn(),
    {
      id: 'keys',
      header: 'Keys',
      priority: 2,
      minWidth: 60,
      align: 'end',
      cell: (r) => String(r.keys_count),
      text: (r) => String(r.keys_count),
      sort: (a, b) => compareNumber(a.keys_count, b.keys_count),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: ref(SECRETS, 'Secret'),
  refresh: () => ({ type: 'RefreshSecrets' }),
  read: readNs('SelectSecret'),
  detailSections: [
    metadataSection(true),
    {
      id: 'type',
      title: 'Type',
      render: ({ row, detail }) => {
        const t = detail?.secret_type ?? row?.secret_type
        return t
          ? { type: 'fields', fields: [{ label: 'Secret type', value: t, mono: true }] }
          : null
      },
    },
    {
      id: 'keys',
      title: 'Keys (values hidden)',
      render: ({ detail }) =>
        detail
          ? {
              type: 'list',
              items: [
                <SecretKeys
                  key="keys"
                  keys={detail.keys}
                  namespace={detail.namespace}
                  name={detail.name}
                />,
              ],
            }
          : null,
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeleteSecret', data: { namespace, name } }),
      'secrets',
    ),
  ],
  selectListEvent: (e) => (e.type === 'SecretsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'SecretDetailsUpdated' ? (d.data as SecretDetails) : undefined,
  rawJson: (d) => d.raw_json,
}

// ── ResourceQuotas ──────────────────────────────────────────────────────────

function QuotaBars({ hard, used }: { hard: Record<string, string>; used: Record<string, string> }) {
  const names = Object.keys(hard).sort()
  if (names.length === 0) return <span className="text-muted-foreground">none</span>
  return (
    <div className="flex flex-col gap-2">
      {names.map((n) => {
        const h = hard[n] ?? ''
        const u = used[n] ?? '0'
        const hn = parseQuantity(h)
        const un = parseQuantity(u)
        const pct = hn && un !== undefined ? Math.min(100, Math.round((un / hn) * 100)) : undefined
        return (
          <div key={n} data-testid={`quota-${n}`} className="flex flex-col gap-0.5">
            <div className="flex justify-between gap-2">
              <span>{n}</span>
              <span>
                {u} / {h}
                {pct !== undefined ? ` (${String(pct)}%)` : ''}
              </span>
            </div>
            {pct !== undefined && (
              <div
                role="progressbar"
                aria-label={`${n} used`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={pct}
                aria-valuetext={`${u} of ${h}`}
                className="h-1.5 w-full bg-muted"
              >
                <div
                  className={pct >= 90 ? 'h-full bg-destructive' : 'h-full bg-accent-neon'}
                  style={{ width: `${String(pct)}%` }}
                />
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

export const resourceQuotas: ResourceDescriptor<ResourceQuotaSummary, ResourceQuotaDetails> = {
  section: QUOTAS,
  kind: 'ResourceQuota',
  label: 'Resource Quotas',
  namespaced: true,
  columns: [
    nameColumn(),
    namespaceColumn(),
    {
      id: 'hard',
      header: 'Limits',
      priority: 1,
      minWidth: 60,
      align: 'end',
      cell: (r) => String(r.hard_limits_count),
      text: (r) => String(r.hard_limits_count),
      sort: (a, b) => compareNumber(a.hard_limits_count, b.hard_limits_count),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: ref(QUOTAS, 'ResourceQuota'),
  refresh: () => ({ type: 'RefreshResourceQuotas' }),
  read: readNs('SelectResourceQuota'),
  detailSections: [
    metadataSection(true),
    {
      id: 'usage',
      title: 'Usage (used / hard)',
      render: ({ detail }) =>
        detail
          ? { type: 'list', items: [<QuotaBars key="q" hard={detail.hard} used={detail.used} />] }
          : null,
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeleteResourceQuota', data: { namespace, name } }),
      'resourcequotas',
    ),
  ],
  selectListEvent: (e) => (e.type === 'ResourceQuotasUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'ResourceQuotaDetailsUpdated' ? (d.data as ResourceQuotaDetails) : undefined,
  rawJson: (d) => d.raw_json,
}

// ── LimitRanges ─────────────────────────────────────────────────────────────

const kv = (m: Record<string, string>) =>
  Object.entries(m)
    .map(([k, v]) => `${k}=${v}`)
    .join(', ') || '—'

export const limitRanges: ResourceDescriptor<LimitRangeSummary, LimitRangeDetails> = {
  section: LIMITS,
  kind: 'LimitRange',
  label: 'Limit Ranges',
  namespaced: true,
  columns: [
    nameColumn(),
    namespaceColumn(),
    {
      id: 'limits',
      header: 'Limits',
      priority: 1,
      minWidth: 60,
      align: 'end',
      cell: (r) => String(r.limits_count),
      text: (r) => String(r.limits_count),
      sort: (a, b) => compareNumber(a.limits_count, b.limits_count),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: ref(LIMITS, 'LimitRange'),
  refresh: () => ({ type: 'RefreshLimitRanges' }),
  read: readNs('SelectLimitRange'),
  detailSections: [
    metadataSection(true),
    {
      id: 'limits',
      title: 'Limits',
      render: ({ detail }) =>
        detail
          ? {
              type: 'list',
              items: detail.limits.map((l, i) => (
                <div key={`${l.limit_type}-${String(i)}`} data-testid={`limit-${l.limit_type}`}>
                  <strong>{l.limit_type}</strong>
                  <div>min: {kv(l.min)}</div>
                  <div>max: {kv(l.max)}</div>
                  <div>default: {kv(l.default)}</div>
                </div>
              )),
            }
          : null,
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeleteLimitRange', data: { namespace, name } }),
      'limitranges',
    ),
  ],
  selectListEvent: (e) => (e.type === 'LimitRangesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'LimitRangeDetailsUpdated' ? (d.data as LimitRangeDetails) : undefined,
  rawJson: (d) => d.raw_json,
}

// ── PodDisruptionBudgets ────────────────────────────────────────────────────

/** Healthy when currently-healthy pods meet the desired count; unknown when the API has not said. */
export function pdbStatus(r: {
  current_healthy: number | null
  desired_healthy: number | null
}): string {
  if (r.current_healthy === null || r.desired_healthy === null) return 'Unknown'
  return r.current_healthy >= r.desired_healthy ? 'Healthy' : 'Disrupted'
}

export const podDisruptionBudgets: ResourceDescriptor<
  PodDisruptionBudgetSummary,
  PodDisruptionBudgetDetails
> = {
  section: PDB,
  kind: 'PodDisruptionBudget',
  label: 'Pod Disruption Budgets',
  namespaced: true,
  columns: [
    nameColumn(),
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 90,
      status: true,
      cell: (r) => <StatusBadge status={pdbStatus(r)} />,
      text: pdbStatus,
      sort: (a, b) => compareStatus(pdbStatus(a), pdbStatus(b)),
    },
    namespaceColumn(),
    {
      id: 'minAvailable',
      header: 'Min available',
      priority: 2,
      minWidth: 100,
      cell: (r) => dash(r.min_available),
      text: (r) => dash(r.min_available),
      sort: (a, b) => compareText(a.min_available ?? '', b.min_available ?? ''),
    },
    {
      id: 'maxUnavailable',
      header: 'Max unavailable',
      priority: 3,
      minWidth: 110,
      cell: (r) => dash(r.max_unavailable),
      text: (r) => dash(r.max_unavailable),
    },
    {
      id: 'healthy',
      header: 'Healthy',
      priority: 3,
      minWidth: 80,
      align: 'end',
      cell: (r) => `${num(r.current_healthy)}/${num(r.desired_healthy)}`,
      text: (r) => `${num(r.current_healthy)}/${num(r.desired_healthy)}`,
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: ref(PDB, 'PodDisruptionBudget'),
  refresh: () => ({ type: 'RefreshPodDisruptionBudgets' }),
  read: readNs('SelectPodDisruptionBudget'),
  detailSections: [
    metadataSection(true),
    {
      id: 'status',
      title: 'Status',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'State', value: <StatusBadge status={pdbStatus(d)} /> },
            { label: 'Currently healthy', value: num(d.current_healthy), mono: true },
            { label: 'Desired healthy', value: num(d.desired_healthy), mono: true },
            ...(detail
              ? [
                  {
                    label: 'Disruptions allowed',
                    value: num(detail.disruptions_allowed),
                    mono: true,
                  },
                  { label: 'Expected pods', value: num(detail.expected_pods), mono: true },
                ]
              : []),
          ],
        }
      },
    },
    {
      id: 'spec',
      title: 'Spec',
      render: ({ row, detail }) => {
        const d = detail ?? row
        if (!d) return null
        return {
          type: 'fields',
          fields: [
            { label: 'Min available', value: dash(d.min_available), mono: true },
            { label: 'Max unavailable', value: dash(d.max_unavailable), mono: true },
          ],
        }
      },
    },
    ...labelSections(),
  ],
  actions: [
    gatedDelete(
      (namespace, name) => ({ type: 'DeletePodDisruptionBudget', data: { namespace, name } }),
      'poddisruptionbudgets',
      'policy',
    ),
  ],
  selectListEvent: (e) => (e.type === 'PodDisruptionBudgetsUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'PodDisruptionBudgetDetailsUpdated'
      ? (d.data as PodDisruptionBudgetDetails)
      : undefined,
  rawJson: (d) => d.raw_json,
}
