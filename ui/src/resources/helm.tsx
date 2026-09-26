/**
 * Helm Releases (spec 005): reads Helm 3's own release-storage Secret/ConfigMap objects
 * directly (no `helm` binary shellout — backend `src/backend/helm.rs`). `read()` also fires
 * `RefreshRelatedResources` so the "owns"/"reconciled-by" relations (US2's origin-link
 * click-through) populate the Related section, mirroring `applications.tsx`.
 *
 * 006-helm-mutate US1: the "Roll back" action on each revision row (`RollbackAction` below)
 * drives `helmMutation` preview -> confirm -> apply through the store, reusing `YamlDiffView`
 * (generalized T017) for the diff and `ConfirmDialog` (unmodified) as the confirm gate. Routed
 * the same way as `YamlEditTab`'s apply flow rather than through the generic bulk
 * `ActionDescriptor`, which is built for row-oriented bulk delete/scale/restart, not a per-row
 * preview-diff-then-confirm flow.
 */
import { useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { GitOpsWarningBanner } from '@/components/panels/GitOpsWarningBanner'
import { YamlDiffView } from '@/components/panels/YamlDiffView'
import { exportFilename } from '@/features/manifest-export/clean'
import { ManifestActions } from '@/features/manifest-export/ManifestActions'
import { StatusBadge } from '@/components/StatusBadge'
import { Button } from '@/components/ui/button'
import { GatedButton } from '@/components/GatedButton'
import { useMutationAccess } from '@/hooks/useMutationAccess'
import type {
  GitOpsOriginRef,
  HelmReleaseDetails,
  HelmReleaseStatus,
  HelmReleaseSummary,
  HelmRevisionSummary,
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
          {
            section: origin.section,
            kind: origin.kind,
            namespace: origin.namespace,
            name: origin.name,
          },
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

/**
 * "Roll back" on one revision row (006-helm-mutate T018/T019): previews the diff
 * (`PreviewHelmRollback`), shows it inline via `YamlDiffView` with a GitOps-ownership warning
 * when `preview.gitops_warning` is set (FR-009/FR-010), then gates the apply
 * (`ApplyHelmRollback`) behind `ConfirmDialog`. Disabled on the currently-deployed revision
 * (nothing to roll back to). Renders its inline preview only under the row it was opened for;
 * `helmMutation` is a single global flow (store/helmMutation.ts), so opening a different row's
 * preview implicitly replaces it.
 */
function RollbackAction({
  namespace,
  release,
  revision,
  currentRevision,
}: {
  namespace: string
  release: string
  revision: HelmRevisionSummary
  currentRevision: number
}) {
  const session = useStore((s) => s.helmMutation)
  const previewHelmRollback = useStore((s) => s.previewHelmRollback)
  const applyHelmRollback = useStore((s) => s.applyHelmRollback)
  const resetHelmMutation = useStore((s) => s.resetHelmMutation)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const access = useMutationAccess({ verb: 'update', resource: 'secrets', namespace })

  const preview = session.preview
  const isThisRevision =
    session.status !== 'idle' &&
    preview?.release === release &&
    preview.namespace === namespace &&
    preview.target_revision === revision.revision

  if (!isThisRevision) {
    const busy = session.status === 'previewing' || session.status === 'applying'
    return (
      <GatedButton
        access={access}
        what="roll back Helm releases"
        data-testid={`helm-rollback-${revision.revision}`}
        size="sm"
        variant="outline"
        disabled={busy || revision.revision === currentRevision}
        onClick={() => previewHelmRollback(namespace, release, revision.revision)}
      >
        Roll back
      </GatedButton>
    )
  }

  return (
    <div className="flex flex-col gap-2 border border-border p-2">
      {session.status === 'previewing' && (
        <p className="text-xs text-muted-foreground">Computing diff…</p>
      )}
      {session.status === 'preview_failed' && (
        <p data-testid="helm-rollback-preview-error" className="text-xs text-destructive">
          {session.lastError}
        </p>
      )}
      {(session.status === 'preview_ready' || session.status === 'applying') && (
        <>
          {preview.gitops_warning && (
            <GitOpsWarningBanner
              origin={preview.gitops_warning}
              testId="helm-rollback-gitops-warning"
              consequence="A rollback may be reverted by the next reconciliation."
            />
          )}
          <YamlDiffView
            currentManifest={preview.current_manifest}
            targetManifest={preview.target_manifest}
          />
          {/* 040 US3 (FR-018): the manifest the rollback restores. */}
          <ManifestActions
            idPrefix="helm-rollback-manifest"
            text={preview.target_manifest}
            filename={exportFilename('helm', [
              { namespace, name: `${release}-rev${String(revision.revision)}` },
            ])}
            what="manifest"
          />
          <div className="flex justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              disabled={session.status === 'applying'}
              onClick={() => resetHelmMutation()}
            >
              Cancel
            </Button>
            <Button
              data-testid="helm-rollback-confirm-open"
              size="sm"
              disabled={session.status === 'applying'}
              onClick={() => setConfirmOpen(true)}
            >
              Roll back to #{revision.revision}
            </Button>
          </div>
        </>
      )}
      {session.status === 'applied' && <p className="text-xs text-status-ok">Rolled back.</p>}
      {session.status === 'apply_failed' && (
        <>
          <p data-testid="helm-rollback-apply-error" className="text-xs text-destructive">
            {session.lastError}
          </p>
          <div className="flex justify-end">
            <Button size="sm" variant="ghost" onClick={() => resetHelmMutation()}>
              Dismiss
            </Button>
          </div>
        </>
      )}

      <ConfirmDialog
        open={confirmOpen}
        title={`Roll back ${release} to revision ${revision.revision}`}
        items={[{ namespace, name: release }]}
        detail={
          preview.gitops_warning
            ? `Managed by ${preview.gitops_warning.controller} — a rollback may be reverted by the next reconciliation. This creates a new revision; it cannot be undone.`
            : 'This creates a new revision pointing at the target manifest. It cannot be undone.'
        }
        destructive
        confirmLabel="Roll back"
        onConfirm={() => {
          setConfirmOpen(false)
          applyHelmRollback(namespace, release, revision.revision, preview.base_revision)
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
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
        {
          label: 'Chart version',
          value: detail?.chart_version ?? row?.chart_version ?? '',
          mono: true,
        },
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
          {
            label: 'Updated',
            value: updated ? `${updated} (${formatAge(updated)} ago)` : '—',
            mono: true,
          },
          { label: 'Degraded', value: degraded ? 'Yes — release storage undecodable' : 'No' },
        ],
      }
    },
  },
  {
    id: 'values-user',
    title: 'Values (user-supplied)',
    render: ({ detail }) =>
      detail ? { type: 'text', text: detail.values_user_supplied || '—' } : null,
  },
  {
    // `id: 'spec'` per the registry's cross-kind convention (`descriptors.test.ts`): the
    // effective values, merged from chart defaults + user overrides, are a Helm release's
    // closest analogue to another kind's spec.
    id: 'spec',
    title: 'Values (effective)',
    render: ({ detail }) =>
      detail ? { type: 'text', text: detail.values_effective || '—' } : null,
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
          <div key={r.revision} className="flex flex-col gap-2">
            <div className="flex items-center gap-2">
              <span className="font-mono text-[0.85em]">#{r.revision}</span>
              <StatusBadge status={helmReleaseStatusLabel(r.status)} />
              <span className="text-[0.85em] text-muted-foreground">{r.chart_version}</span>
              <span className="text-[0.85em] text-muted-foreground">{r.updated}</span>
              <RollbackAction
                namespace={detail.namespace}
                release={detail.name}
                revision={r}
                currentRevision={detail.revision}
              />
            </div>
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
        return {
          type: 'fields',
          fields: [{ label: 'Origin', value: 'Not managed by a GitOps controller' }],
        }
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
      sort: (a, b) =>
        compareStatus(helmReleaseStatusLabel(a.status), helmReleaseStatusLabel(b.status)),
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
