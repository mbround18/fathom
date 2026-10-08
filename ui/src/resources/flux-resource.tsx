/**
 * Flux GitOps resource (spec 005, verbs in 049): reached from the Flux pane's rows and from a
 * Helm release's origin-link click-through (`related.tsx`'s `RelatedResourceLink` /
 * `GitOpsOriginRef`), never listed in the navigator — `nav.ts` intentionally has no entry for
 * `section_keys::flux_resource()`. Reuses the backend's existing Flux discovery/summarize logic
 * (`SelectFluxResource`, no new poll loop). The overview doubles as this section's list
 * (`handlers/system.ts`), which is how the inspector's action menu finds the row it acts on.
 */
import { StatusBadge } from '@/components/StatusBadge'
import type { FluxResourceSummary } from '@/protocol/types'
import { FLUX_RESOURCE_SECTION } from '@/store/keys'
import { useStore } from '@/store'

import type { ActionDescriptor, DetailSection, ResourceDescriptor } from './types'

const SECTION = FLUX_RESOURCE_SECTION

/** The CRD each Flux kind lives in, for the RBAC question an action asks (`accessCheck`). */
const CRD: Record<string, { resource: string; group: string }> = {
  GitRepository: { resource: 'gitrepositories', group: 'source.toolkit.fluxcd.io' },
  OCIRepository: { resource: 'ocirepositories', group: 'source.toolkit.fluxcd.io' },
  HelmRepository: { resource: 'helmrepositories', group: 'source.toolkit.fluxcd.io' },
  Bucket: { resource: 'buckets', group: 'source.toolkit.fluxcd.io' },
  Kustomization: { resource: 'kustomizations', group: 'kustomize.toolkit.fluxcd.io' },
  HelmRelease: { resource: 'helmreleases', group: 'helm.toolkit.fluxcd.io' },
}

export function isFluxSource(kind: string): boolean {
  return kind !== 'Kustomization' && kind !== 'HelmRelease'
}

const accessCheck = (verb: string) => (row: FluxResourceSummary) => {
  const crd = CRD[row.kind] ?? { resource: row.kind.toLowerCase() + 's', group: '' }
  return { verb, resource: crd.resource, group: crd.group, namespace: row.namespace }
}

/** The Kustomizations and HelmReleases whose source this resource is, from the overview. */
export function fluxDependents(
  r: FluxResourceSummary,
  overview = useStore.getState().flux.overview,
): FluxResourceSummary[] {
  if (!overview || !isFluxSource(r.kind)) return []
  const ref = `${r.kind}/${r.name}`
  return [...overview.kustomizations, ...overview.helmreleases].filter((d) => d.source_ref === ref)
}

const SHOWN = 12

function Reach({ title, lines, none }: { title: string; lines: string[]; none: string }) {
  if (lines.length === 0) return <p>{none}</p>
  const rest = lines.length - SHOWN
  return (
    <div data-testid="flux-reach">
      <p>{title}</p>
      <ul className="mt-1 max-h-40 overflow-y-auto font-mono text-xs">
        {lines.slice(0, SHOWN).map((l) => (
          <li key={l}>{l}</li>
        ))}
        {rest > 0 && <li className="text-muted-foreground">and {String(rest)} more</li>}
      </ul>
    </div>
  )
}

/** What the verb reaches: a Kustomization's inventory, a HelmRelease's release, a source's users. */
function reach(r: FluxResourceSummary, stops: boolean) {
  if (isFluxSource(r.kind)) {
    const users = fluxDependents(r).map((d) => `${d.kind} ${d.namespace}/${d.name}`)
    return (
      <Reach
        title={
          stops
            ? 'These stop receiving new artifacts from it until it is resumed:'
            : 'These pull from it:'
        }
        lines={users}
        none="Nothing in the overview pulls from this source."
      />
    )
  }
  return (
    <Reach
      title={
        stops
          ? 'Flux stops applying these until it is resumed, so a hand edit sticks:'
          : 'It applies:'
      }
      lines={r.managed}
      none="Flux reports nothing applied by it yet."
    />
  )
}

const toggle = (suspend: boolean): ActionDescriptor<FluxResourceSummary> => ({
  id: suspend ? 'suspend' : 'resume',
  label: suspend ? 'Suspend' : 'Resume',
  done: suspend ? 'Suspended' : 'Resumed',
  doing: suspend ? 'Suspending' : 'Resuming',
  prompt: 'confirm',
  accessCheck: accessCheck('patch'),
  applies: (r) => r.suspended !== suspend,
  detail: ([r]) => (r ? reach(r, suspend) : undefined),
  plan: (rows) =>
    rows
      .filter((r) => r.suspended !== suspend)
      .map((r) => ({
        cmd: {
          type: 'SuspendFluxResource',
          data: { kind: r.kind, namespace: r.namespace, name: r.name, suspend },
        },
        rows: [r],
      })),
})

const reconcile = (withSource: boolean): ActionDescriptor<FluxResourceSummary> => ({
  id: withSource ? 'reconcile-source' : 'reconcile',
  label: withSource ? 'Reconcile with source' : 'Reconcile',
  done: 'Reconcile requested for',
  doing: 'Requesting reconcile of',
  prompt: 'confirm',
  accessCheck: accessCheck('patch'),
  applies: (r) => !withSource || !isFluxSource(r.kind),
  detail: ([r]) => {
    if (!r) return undefined
    return (
      <div className="space-y-2">
        {r.suspended && (
          <p data-testid="flux-suspended-warning" className="text-amber-400">
            It is suspended: Flux ignores reconcile requests until it is resumed.
          </p>
        )}
        <p>
          {withSource
            ? `Asks Flux to fetch ${r.source_ref ?? 'its source'} first, then reconcile this ${r.kind} now, outside its ${r.interval || 'usual'} interval.`
            : `Asks Flux to reconcile this ${r.kind} now, outside its ${r.interval || 'usual'} interval.`}
        </p>
      </div>
    )
  },
  plan: (rows) =>
    rows.map((r) => ({
      cmd: {
        type: 'ReconcileFluxResource',
        data: { kind: r.kind, namespace: r.namespace, name: r.name, with_source: withSource },
      },
      rows: [r],
    })),
})

const fluxResourceSections: DetailSection<FluxResourceSummary, FluxResourceSummary>[] = [
  {
    id: 'details',
    title: 'Details',
    render: ({ row, detail }) => {
      const d = detail ?? row
      if (!d) return null
      return {
        type: 'fields',
        fields: [
          { label: 'Kind', value: d.kind, mono: true },
          { label: 'Name', value: `${d.namespace}/${d.name}`, mono: true },
          { label: 'Ready', value: <StatusBadge status={d.ready} /> },
          { label: 'Message', value: d.message || '—' },
          { label: 'Revision', value: d.revision || '—', mono: true },
          { label: 'Source', value: d.source_ref ?? '—', mono: true },
          { label: 'Suspended', value: d.suspended ? 'Yes' : 'No' },
          { label: 'Interval', value: d.interval || '—', mono: true },
          { label: 'Last transition', value: d.last_transition || '—', mono: true },
        ],
      }
    },
  },
  {
    id: 'manages',
    title: 'Manages',
    render: ({ row, detail }) => {
      const d = detail ?? row
      if (!d) return null
      const lines = isFluxSource(d.kind)
        ? fluxDependents(d).map((u) => `${u.kind} ${u.namespace}/${u.name}`)
        : d.managed
      return {
        type: 'fields',
        fields: lines.length
          ? lines.map((l) => ({ label: '', value: l, mono: true }))
          : [{ label: '', value: isFluxSource(d.kind) ? 'No dependents' : 'Nothing yet' }],
      }
    },
  },
]

export const fluxResource: ResourceDescriptor<FluxResourceSummary, FluxResourceSummary> = {
  section: SECTION,
  kind: 'FluxResource',
  label: 'Flux Resource',
  namespaced: true,
  columns: [],
  rowKey: (r) => `${r.namespace}/${r.name}`,
  ref: (r) => ({ section: SECTION, kind: r.kind, namespace: r.namespace, name: r.name }),
  // No list command exists for this kind (US3: a deleted origin resolves to
  // `FluxResourceNotFound`, not a list refresh); the overview scan is the nearest refresh.
  refresh: () => ({ type: 'RefreshFlux' }),
  read: (ref) => [
    {
      type: 'SelectFluxResource',
      data: { kind: ref.kind, namespace: ref.namespace ?? '', name: ref.name },
    },
  ],
  detailSections: fluxResourceSections,
  // 049: Suspend or Resume (whichever `applies`), Reconcile, and "with source" where there is one.
  actions: [toggle(true), toggle(false), reconcile(false), reconcile(true)],
  selectListEvent: () => undefined,
  selectDetail: (d) =>
    d.eventType === 'FluxResourceDetailsUpdated' ? (d.data as FluxResourceSummary) : undefined,
}
