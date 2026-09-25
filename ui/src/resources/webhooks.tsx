/**
 * Mutating/ValidatingWebhookConfigurations (batch D, cluster-scoped). The typed detail carries
 * only name/failurePolicy/sideEffects per hook, so rules and the service/url target are read
 * from `raw_json` (defensively: missing fields simply render as absent).
 */
import type {
  MutatingWebhookConfigurationDetails,
  MutatingWebhookConfigurationSummary,
  ValidatingWebhookConfigurationDetails,
  ValidatingWebhookConfigurationSummary,
} from '@/protocol/types'

import { arr, labelSections, metadataSection, obj, parseJson, str } from './configCommon'
import { compareNumber, compareText } from './search'
import type { ActionDescriptor, ResourceDescriptor } from './types'

export interface HookView {
  name: string
  failurePolicy: string
  sideEffects: string
  target: string
  rules: string[]
}

const csv = (v: unknown) =>
  arr(v)
    .map((x) => str(x) || 'core')
    .join(', ') || '*'

export function hookViews(raw: string): HookView[] {
  return arr(parseJson(raw).webhooks)
    .map(obj)
    .map((h) => {
      const cc = obj(h.clientConfig)
      const svc = obj(cc.service)
      const port = typeof svc.port === 'number' ? `:${String(svc.port)}` : ''
      const target = str(cc.url)
        ? str(cc.url)
        : str(svc.name)
          ? `service ${str(svc.namespace)}/${str(svc.name)}${port}${str(svc.path)}`
          : '—'
      return {
        name: str(h.name),
        failurePolicy: str(h.failurePolicy) || '—',
        sideEffects: str(h.sideEffects) || '—',
        target,
        rules: arr(h.rules)
          .map(obj)
          .map(
            (r) =>
              `${csv(r.operations)} ${csv(r.apiGroups)}/${csv(r.apiVersions)} ${csv(r.resources)}`,
          ),
      }
    })
}

interface Hook {
  name: string
  failure_policy: string | null
  side_effects: string | null
}

function HookCard({ h, v }: { h: Hook; v?: HookView }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5" data-testid="webhook-hook">
      <span className="truncate font-mono">{h.name}</span>
      <span className="text-muted-foreground">
        failurePolicy {h.failure_policy ?? v?.failurePolicy ?? '—'} · sideEffects{' '}
        {h.side_effects ?? v?.sideEffects ?? '—'}
      </span>
      {v && <span className="font-mono break-all">{v.target}</span>}
      {v?.rules.map((r, i) => (
        <span key={`${r}-${String(i)}`} className="font-mono break-all text-muted-foreground">
          {r}
        </span>
      ))}
    </div>
  )
}

interface CfgSummary {
  name: string
  webhooks_count: number
}
interface CfgDetails {
  name: string
  creation_timestamp: string
  labels: Record<string, string>
  annotations: Record<string, string>
  webhooks: Hook[]
  raw_json: string
}

interface Opts {
  section: string
  kind: string
  label: string
  resource: string
  refresh: 'RefreshMutatingWebhookConfigurations' | 'RefreshValidatingWebhookConfigurations'
  select: 'SelectMutatingWebhookConfiguration' | 'SelectValidatingWebhookConfiguration'
  del: 'DeleteMutatingWebhookConfiguration' | 'DeleteValidatingWebhookConfiguration'
  listEvent: string
  readEvent: string
}

function webhookDescriptor<S extends CfgSummary, D extends CfgDetails>(
  o: Opts,
): ResourceDescriptor<S, D> {
  const del: ActionDescriptor<S> = {
    id: 'delete',
    label: 'Delete',
    done: 'Deleted',
    doing: 'Deleting',
    prompt: 'confirm',
    destructive: true,
    bulk: true,
    accessCheck: () => ({
      verb: 'delete',
      group: 'admissionregistration.k8s.io',
      resource: o.resource,
      namespace: null,
    }),
    plan: (rows) => rows.map((r) => ({ cmd: { type: o.del, data: { name: r.name } }, rows: [r] })),
  }
  return {
    section: o.section,
    kind: o.kind,
    label: o.label,
    namespaced: false,
    columns: [
      {
        id: 'name',
        header: 'Name',
        priority: 1,
        minWidth: 150,
        grow: 3,
        cell: (r) => r.name,
        text: (r) => r.name,
        sort: (a, b) => compareText(a.name, b.name),
      },
      {
        id: 'webhooks',
        header: 'Webhooks',
        priority: 1,
        minWidth: 84,
        align: 'end',
        cell: (r) => String(r.webhooks_count),
        text: (r) => String(r.webhooks_count),
        sort: (a, b) => compareNumber(a.webhooks_count, b.webhooks_count),
      },
    ],
    rowKey: (r) => r.name,
    ref: (r) => ({ section: o.section, kind: o.kind, namespace: null, name: r.name }),
    refresh: () => ({ type: o.refresh }),
    read: (ref) => [{ type: o.select, data: { name: ref.name } }, { type: 'RefreshEvents' }],
    detailSections: [
      metadataSection<S, D>(false),
      {
        id: 'webhooks',
        title: 'Webhooks',
        render: ({ detail }) => {
          if (!detail) return null
          if (detail.webhooks.length === 0) return { type: 'text', text: 'No webhooks defined.' }
          const views = hookViews(detail.raw_json)
          return {
            type: 'list',
            items: detail.webhooks.map((h, i) => (
              <HookCard
                key={`${h.name}-${String(i)}`}
                h={h}
                v={views.find((v) => v.name === h.name)}
              />
            )),
          }
        },
      },
      ...labelSections<S, D>(),
    ],
    actions: [del],
    selectListEvent: (e) => (e.type === o.listEvent ? (e.data as S[]) : undefined),
    selectDetail: (d) => (d.eventType === o.readEvent ? (d.data as D) : undefined),
    rawJson: (d) => d.raw_json,
  }
}

export const mutatingwebhooks = webhookDescriptor<
  MutatingWebhookConfigurationSummary,
  MutatingWebhookConfigurationDetails
>({
  section: 'config/mutatingwebhooks',
  kind: 'MutatingWebhookConfiguration',
  label: 'Mutating Webhook Configurations',
  resource: 'mutatingwebhookconfigurations',
  refresh: 'RefreshMutatingWebhookConfigurations',
  select: 'SelectMutatingWebhookConfiguration',
  del: 'DeleteMutatingWebhookConfiguration',
  listEvent: 'MutatingWebhookConfigurationsUpdated',
  readEvent: 'MutatingWebhookConfigurationDetailsUpdated',
})

export const validatingwebhooks = webhookDescriptor<
  ValidatingWebhookConfigurationSummary,
  ValidatingWebhookConfigurationDetails
>({
  section: 'config/validatingwebhooks',
  kind: 'ValidatingWebhookConfiguration',
  label: 'Validating Webhook Configurations',
  resource: 'validatingwebhookconfigurations',
  refresh: 'RefreshValidatingWebhookConfigurations',
  select: 'SelectValidatingWebhookConfiguration',
  del: 'DeleteValidatingWebhookConfiguration',
  listEvent: 'ValidatingWebhookConfigurationsUpdated',
  readEvent: 'ValidatingWebhookConfigurationDetailsUpdated',
})
