/**
 * Helm Releases (spec 005): reads Helm 3's own release-storage Secret/ConfigMap objects
 * directly (no `helm` binary shellout — backend `src/backend/helm.rs`). `read()` also fires
 * `RefreshRelatedResources` so the "owns"/"reconciled-by" relations (US2's origin-link
 * click-through) populate the Related section, mirroring `applications.tsx`.
 */
import { StatusBadge } from '@/components/StatusBadge'
import type {
  GitOpsOriginRef,
  HelmReleaseDetails,
  HelmReleaseStatus,
  HelmReleaseSummary,
} from '@/protocol/types'
import { useStore } from '@/store'

import { formatAge } from './format'
import { relatedResourcesBlock } from './related'
import { compareNumber, compareStatus, compareText } from './search'
import type { DetailSection, ResourceDescriptor } from './types'

const SECTION = 'helm/releases'

export function helmReleaseStatusLabel(status: HelmReleaseStatus): string {
  switch (status) {
    case 'PendingInstall':
      return 'Pending install'
    case 'PendingUpgrade':
      return 'Pending upgrade'
    case 'PendingRollback':
      return 'Pending rollback'
    default:
      return status
  }
}

/** Origin-link click-through (spec 005 US2): opens the Flux resource's own inspector, keyed by
 * the real k8s kind (`o.kind`, "Kustomization"/"HelmRelease") so `readKeyOf`'s cache lookup
 * (`store/handlers/read.ts`) matches the reply's payload `kind`, not the fixed section kind. */
function OriginLink({ origin }: { origin: GitOpsOriginRef }) {
  const openInspector = useStore((s) => s.openInspector)
  return (
    <button
      type="button"
      onClick={() => {
        openInspector(
          { section: origin.section, kind: origin.kind, namespace: origin.namespace, name: origin.name },
          undefined,
          true,
        )
      }}
      className="text-left hover:text-accent-neon"
    >
      {origin.namespace}/{origin.name}
    </button>
  )
}

const helmReleaseSections: DetailSection<HelmReleaseSummary, HelmReleaseDetails>[] = [
  {
    id: 'metadata',
    title: 'Metadata',
    render: ({ row, detail }) => {
      if (!row && !detail) return null
      const fields = [
        { label: 'Name', value: detail?.name ?? row?.name ?? '', mono: true },
        { label: 'Namespace', value: detail?.namespace ?? row?.namespace ?? '', mono: true },
        { label: 'Chart', value: detail?.chart ?? row?.chart ?? '', mono: true },
        { label: 'Chart version', value: detail?.chart_version ?? row?.chart_version ?? '', mono: true },
        { label: 'App version', value: detail?.app_version ?? row?.app_version ?? '—', mono: true },
        { label: 'Revision', value: String(detail?.revision ?? row?.revision ?? ''), mono: true },
      ]
      if (detail?.description) {
        fields.push({ label: 'Description', value: detail.description, mono: false })
      }
      return { type: 'fields', fields }
    },
  },
  {
    id: 'status',
    title: 'Status',
    render: ({ row, detail }) => {
      const status = detail?.status ?? row?.status
      const degraded = detail?.degraded ?? row?.degraded
      const updated = detail?.updated ?? row?.updated
      if (!status) return null
      return {
        type: 'fields',
        fields: [
          { label: 'Status', value: <StatusBadge status={helmReleaseStatusLabel(status)} /> },
          { label: 'Updated', value: updated ? `${updated} (${formatAge(updated)} ago)` : '—', mono: true },
          { label: 'Degraded', value: degraded ? 'Yes — release storage undecodable' : 'No' },
        ],
      }
    },
  },
  {
    id: 'values-user',
    title: 'Values (user-supplied)',
    render: ({ detail }) => (detail ? { type: 'text', text: detail.values_user_supplied || '—' } : null),
  },
  {
    // `id: 'spec'` per the registry's cross-kind convention (`descriptors.test.ts`): the
    // effective values, merged from chart defaults + user overrides, are a Helm release's
    // closest analogue to another kind's spec.
    id: 'spec',
    title: 'Values (effective)',
    render: ({ detail }) => (detail ? { type: 'text', text: detail.values_effective || '—' } : null),
  },
  {
    id: 'manifest',
    title: 'Manifest',
    render: ({ detail }) => (detail ? { type: 'text', text: detail.manifest || '—' } : null),
  },
  {
    id: 'revisions',
    title: 'Revisions',
    render: ({ detail }) => {
      if (!detail) return null
      if (detail.revisions.length === 0) return { type: 'list', items: ['No revisions'] }
      return {
        type: 'list',
        items: detail.revisions.map((r) => (
          <div key={r.revision} className="flex items-center gap-2">
            <span className="font-mono text-[0.85em]">#{r.revision}</span>
            <StatusBadge status={helmReleaseStatusLabel(r.status)} />
            <span className="text-muted-foreground text-[0.85em]">{r.chart_version}</span>
            <span className="text-muted-foreground text-[0.85em]">{r.updated}</span>
          </div>
        )),
      }
    },
  },
  {
    id: 'origin',
    title: 'GitOps Origin',
    render: ({ detail }) => {
      if (!detail) return null
      if (!detail.origin) {
        return { type: 'fields', fields: [{ label: 'Origin', value: 'Not managed by a GitOps controller' }] }
      }
      const o = detail.origin
      return {
        type: 'fields',
        fields: [
          { label: 'Controller', value: o.controller },
          { label: 'Kind', value: o.kind, mono: true },
          { label: 'Name', value: <OriginLink origin={o} />, mono: true },
          { label: 'Ready', value: <StatusBadge status={o.ready} /> },
          { label: 'Message', value: o.message ?? '—' },
        ],
      }
    },
  },
  {
    id: 'labels',
    title: 'Labels',
    // `HelmReleaseDetails` has no labels of its own (it is synthesized from Helm's release
    // storage, not a single live k8s object) — rendered as an empty map for consistency with
    // every other descriptor's sections, same precedent as `applications.tsx`.
    render: ({ detail }) => (detail ? { type: 'map', entries: {} } : null),
  },
  {
    id: 'annotations',
    title: 'Annotations',
    render: ({ detail }) => (detail ? { type: 'map', entries: {} } : null),
  },
  {
    id: 'related',
    title: 'Related',
    render: ({ detail }) => (detail ? relatedResourcesBlock(detail.related) : null),
  },
]

export const helm: ResourceDescriptor<HelmReleaseSummary, HelmReleaseDetails> = {
  section: SECTION,
  kind: 'HelmRelease',
  label: 'Helm Releases',
  namespaced: true,
  columns: [
    {
      id: 'name',
      header: 'Name',
      priority: 1,
      minWidth: 140,
      grow: 2,
      cell: (r) => r.name,
      text: (r) => r.name,
      sort: (a, b) => compareText(a.name, b.name),
    },
    {
      id: 'namespace',
      header: 'Namespace',
      priority: 2,
      minWidth: 112,
      cell: (r) => r.namespace,
      text: (r) => r.namespace,
      sort: (a, b) => compareText(a.namespace, b.namespace),
    },
    {
      id: 'chart',
      header: 'Chart',
      priority: 2,
      minWidth: 120,
      cell: (r) => r.chart,
      text: (r) => r.chart,
      sort: (a, b) => compareText(a.chart, b.chart),
    },
    {
      id: 'chart_version',
      header: 'Chart Version',
      priority: 3,
      minWidth: 100,
      cell: (r) => r.chart_version,
      text: (r) => r.chart_version,
      sort: (a, b) => compareText(a.chart_version, b.chart_version),
    },
    {
      id: 'app_version',
      header: 'App Version',
      priority: 3,
      minWidth: 100,
      cell: (r) => r.app_version ?? '—',
      text: (r) => r.app_version ?? '',
      sort: (a, b) => compareText(a.app_version ?? '', b.app_version ?? ''),
    },
    {
      id: 'revision',
      header: 'Revision',
      priority: 3,
      minWidth: 72,
      align: 'end',
      cell: (r) => String(r.revision),
      text: (r) => String(r.revision),
      sort: (a, b) => compareNumber(a.revision, b.revision),
    },
    {
      id: 'status',
      header: 'Status',
      priority: 1,
      minWidth: 108,
      status: true,
      cell: (r) => <StatusBadge status={helmReleaseStatusLabel(r.status)} />,
      text: (r) => helmReleaseStatusLabel(r.status),
      sort: (a, b) => compareStatus(helmReleaseStatusLabel(a.status), helmReleaseStatusLabel(b.status)),
    },
    {
      id: 'updated',
      header: 'Last Updated',
      priority: 4,
      minWidth: 96,
      align: 'end',
      cell: (r) => (r.updated ? formatAge(r.updated) : '—'),
      text: (r) => (r.updated ? formatAge(r.updated) : ''),
      sort: (a, b) => Date.parse(a.updated) - Date.parse(b.updated),
    },
  ],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: 'HelmRelease', namespace: r.namespace, name: r.name }),
  refresh: () => ({ type: 'RefreshHelmReleases' }),
  read: (ref) => [
    { type: 'SelectHelmRelease', data: { namespace: ref.namespace ?? '', name: ref.name } },
    { type: 'RefreshEvents' },
    {
      type: 'RefreshRelatedResources',
      data: { section: SECTION, kind: 'HelmRelease', namespace: ref.namespace, name: ref.name },
    },
  ],
  detailSections: helmReleaseSections,
  actions: [],
  selectListEvent: (e) => (e.type === 'HelmReleasesUpdated' ? e.data : undefined),
  selectDetail: (d) =>
    d.eventType === 'HelmReleaseDetailsUpdated' ? (d.data as HelmReleaseDetails) : undefined,
  // Deliberately no `rawJson`: Values/Manifest are shown as read-only `text` blocks above rather
  // than through `YamlEditTab`, since `rawJson` on a descriptor also turns on that tab's
  // apply/edit path (`descriptor.rawJson !== undefined` -> editable in Inspector.tsx) — out of
  // scope for this 100%-read-only feature (constitution: no new mutations). Same precedent as
  // `applications.tsx`.
}
