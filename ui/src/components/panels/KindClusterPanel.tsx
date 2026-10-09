/**
 * "Local Clusters" section of Cluster Settings (030-kind-cluster-provisioning): prerequisites,
 * existing kind clusters, and create/delete through the shared `ConfirmDialog`.
 *
 * The backend owns every fact shown here (readiness, inventory, progress). Nothing is sent
 * that mutates the host until the operator confirms; a delete row is removed only once the
 * backend reports success. All testids are prefixed `kind-`.
 */
import { useEffect, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import { ClusterNodes } from '@/features/containers/ClusterNodes'
import { useBackingContainers } from '@/features/containers/useBackingContainers'
import { exportFilename } from '@/features/manifest-export/clean'
import { ManifestActions } from '@/features/manifest-export/ManifestActions'
import { openExternalLink } from '@/lib/open-external'
import type { KindClusterProfileId } from '@/protocol/types'
import { useStore, type KindOperation } from '@/store'

import { KIND_BARE_CONFIG, validateKindClusterName } from './kind-validation'

const INSTALL_URL = 'https://kind.sigs.k8s.io/docs/user/quick-start/#installation'

function newRequestId(): string {
  return `kind-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

function PrerequisitesBanner() {
  const p = useStore((s) => s.kind.prerequisites)
  const request = useStore((s) => s.request)
  const refresh = () => void request({ type: 'RefreshKindPrerequisites' }).catch(() => undefined)

  let body: React.ReactNode
  if (!p) body = <span>Checking your machine…</span>
  else if (p.ready)
    body = (
      <span data-testid="kind-ready">
        Ready — {p.kind_version ?? 'kind'} with {p.runtime_kind}.
      </span>
    )
  else if (!p.kind_installed)
    body = (
      <span data-testid="kind-missing">
        kind is not installed.{' '}
        <a
          className="underline"
          href={INSTALL_URL}
          target="_blank"
          rel="noreferrer"
          onClick={openExternalLink}
        >
          Install kind
        </a>{' '}
        and refresh.
      </span>
    )
  else if (!p.runtime_installed)
    body = (
      <span data-testid="kind-runtime-missing">
        No container runtime found. Install Docker or Podman, then refresh.
      </span>
    )
  else
    body = (
      <span data-testid="kind-runtime-stopped">
        {p.runtime_kind === 'podman' ? 'Podman' : 'Docker'} is installed but not running. Start it,
        then refresh.
      </span>
    )

  return (
    <div className="flex items-center justify-between gap-2 text-xs" data-testid="kind-prereqs">
      {body}
      <Button size="sm" variant="outline" data-testid="kind-prereqs-refresh" onClick={refresh}>
        Refresh
      </Button>
    </div>
  )
}

/**
 * Seconds since the operation view mounted, ticking once a second while `active`. kind prints
 * nothing for ~10 s while the control plane starts (measured live, T050), so a moving counter
 * keeps the view from looking frozen between stage lines (SC-004).
 */
function useElapsedSeconds(active: boolean): number {
  const [startedAt] = useState(() => Date.now())
  const [now, setNow] = useState(startedAt)
  useEffect(() => {
    if (!active) return
    const id = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [active])
  return Math.max(0, Math.floor((now - startedAt) / 1000))
}

function ProgressView({ op }: { op: KindOperation }) {
  const request = useStore((s) => s.request)
  const elapsed = useElapsedSeconds(op.status === 'in-progress')
  return (
    <div
      className="flex items-center justify-between gap-2 text-xs"
      data-testid="kind-progress"
      data-status={op.status}
      role="status"
    >
      <span>
        {op.kind === 'create' ? 'Creating' : 'Deleting'} {op.clusterName}:{' '}
        {op.status === 'in-progress' && (op.stage ?? 'Starting…')}
        {op.status === 'in-progress' && (
          <span
            className="ml-1 text-muted-foreground tabular-nums"
            data-testid="kind-progress-elapsed"
          >
            ({elapsed}s)
          </span>
        )}
        {op.status === 'complete' && 'done'}
        {op.status === 'cancelled' && 'cancelled'}
        {op.status === 'failed' && (
          <span className="text-status-err" data-testid="kind-progress-error">
            failed — {op.error}
          </span>
        )}
      </span>
      {op.status === 'in-progress' && op.kind === 'create' && (
        <Button
          size="sm"
          variant="outline"
          data-testid="kind-cancel"
          onClick={() =>
            void request({
              type: 'CancelCreateKindCluster',
              data: { request_id: op.requestId },
            }).catch(() => undefined)
          }
        >
          Cancel
        </Button>
      )}
    </div>
  )
}

export function KindClusterPanel() {
  const request = useStore((s) => s.request)
  const prereq = useStore((s) => s.kind.prerequisites)
  const clusters = useStore((s) => s.kind.clusters)
  const operations = useStore((s) => s.kind.operations)
  const activeContext = useStore((s) => s.session.activeContext)

  const [name, setName] = useState('')
  const [profile, setProfile] = useState<KindClusterProfileId>('bare')
  const [customYaml, setCustomYaml] = useState('')
  const [confirmingCreate, setConfirmingCreate] = useState(false)
  const [deleting, setDeleting] = useState<string | undefined>(undefined)

  useBackingContainers()

  useEffect(() => {
    request({ type: 'RefreshKindPrerequisites' }).catch(() => undefined)
    request({ type: 'RefreshKindClusters' }).catch(() => undefined)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const existing = (clusters ?? []).map((c) => c.name)
  const nameError = name === '' ? null : validateKindClusterName(name, existing)
  const yamlError =
    profile === 'custom_yaml' && customYaml.trim() === '' ? 'Paste a kind config.' : null
  const ready = prereq?.ready === true
  const canSubmit = ready && name !== '' && nameError === null && yamlError === null

  const busy = Object.values(operations).some((o) => o.status === 'in-progress')
  const sorted = Object.values(operations).sort((a, b) => a.requestId.localeCompare(b.requestId))

  const startOperation = (op: KindOperation) =>
    useStore.setState((s) => ({
      kind: { ...s.kind, operations: { ...s.kind.operations, [op.requestId]: op } },
    }))

  const confirmCreate = () => {
    const requestId = newRequestId()
    setConfirmingCreate(false)
    startOperation({ requestId, clusterName: name, kind: 'create', status: 'in-progress' })
    void request({
      type: 'CreateKindCluster',
      data: {
        request_id: requestId,
        name,
        profile,
        custom_yaml: profile === 'custom_yaml' ? customYaml : null,
      },
    }).catch(() => undefined)
    setName('')
  }

  const confirmDelete = () => {
    if (!deleting) return
    const requestId = newRequestId()
    startOperation({ requestId, clusterName: deleting, kind: 'delete', status: 'in-progress' })
    void request({
      type: 'DeleteKindCluster',
      data: { request_id: requestId, name: deleting },
    }).catch(() => undefined)
    setDeleting(undefined)
  }

  return (
    <fieldset className="flex flex-col gap-3" data-testid="kind-clusters">
      <legend className="text-xs font-semibold tracking-wider uppercase">
        Local Clusters (kind)
      </legend>

      <PrerequisitesBanner />

      <div className="flex items-center justify-between text-xs">
        <span className="font-medium">Existing clusters</span>
        <Button
          size="sm"
          variant="outline"
          data-testid="kind-list-refresh"
          onClick={() => void request({ type: 'RefreshKindClusters' }).catch(() => undefined)}
        >
          Refresh
        </Button>
      </div>

      {clusters && clusters.length === 0 && (
        <p className="text-xs text-muted-foreground" data-testid="kind-empty">
          No local clusters yet — create one below.
        </p>
      )}
      {clusters && clusters.length > 0 && (
        <ul className="flex flex-col gap-1 text-xs" data-testid="kind-list">
          {clusters.map((c) => (
            <li
              key={c.name}
              className="flex flex-col gap-1"
              data-testid="kind-row"
              data-cluster={c.name}
            >
              <div className="flex items-center justify-between gap-2">
                <span>
                  {c.name} <span className="text-muted-foreground">({c.status})</span>
                  {activeContext === c.context_name && (
                    <span className="ml-1" data-testid="kind-selected-badge">
                      · currently selected
                    </span>
                  )}
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  data-testid="kind-delete"
                  disabled={c.status === 'in_progress' || !ready}
                  onClick={() => setDeleting(c.name)}
                >
                  Delete
                </Button>
              </div>
              {/* 048 US4: a kind node is a container on this daemon, and the cluster list is the
                  only place an operator sees the cluster as a whole. Both sides read the same
                  link, so neither can claim a cluster is fine while its nodes are stopped. */}
              <ClusterNodes cluster={c.name} />
            </li>
          ))}
        </ul>
      )}

      <div className="flex flex-col gap-2 text-xs">
        <span className="font-medium">Create a cluster</span>
        <input
          className="rounded border bg-transparent px-2 py-1"
          data-testid="kind-name-input"
          aria-label="Cluster name"
          placeholder="my-cluster"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        {nameError && (
          <p role="alert" className="text-status-err" data-testid="kind-name-error">
            {nameError}
          </p>
        )}
        <label className="flex items-center gap-2">
          Profile
          <select
            data-testid="kind-profile"
            className="rounded border bg-transparent px-1 py-0.5"
            value={profile}
            onChange={(e) => setProfile(e.target.value as KindClusterProfileId)}
          >
            <option value="bare">Bare single-node</option>
            <option value="custom_yaml">Custom (paste kind config YAML)</option>
          </select>
        </label>
        {profile === 'custom_yaml' && (
          <textarea
            className="rounded border bg-transparent px-2 py-1 font-mono"
            data-testid="kind-yaml"
            aria-label="kind config YAML"
            rows={6}
            value={customYaml}
            onChange={(e) => setCustomYaml(e.target.value)}
          />
        )}
        <div>
          <Button
            size="sm"
            data-testid="kind-create"
            disabled={!canSubmit || busy}
            onClick={() => setConfirmingCreate(true)}
          >
            Create cluster
          </Button>
        </div>
      </div>

      {sorted.map((op) => (
        <ProgressView key={op.requestId} op={op} />
      ))}

      <ConfirmDialog
        open={confirmingCreate}
        title="Create local cluster"
        items={[{ namespace: null, name }]}
        detail={
          <div className="flex flex-col gap-2">
            <span data-testid="kind-create-summary">
              Creates kind cluster “{name}” — profile:{' '}
              {profile === 'bare' ? 'bare single-node' : 'custom YAML'}, nodes:{' '}
              {profile === 'bare' ? 1 : 'as defined by your config'}. This adds a kubeconfig context
              named kind-{name}.
            </span>
            <pre
              data-testid="kind-config-preview"
              className="max-h-48 overflow-auto border border-border bg-background p-2 font-mono text-xs whitespace-pre text-foreground"
            >
              {profile === 'bare' ? KIND_BARE_CONFIG : customYaml}
            </pre>
            <ManifestActions
              idPrefix="kind-config"
              what="kind config"
              text={profile === 'bare' ? KIND_BARE_CONFIG : customYaml}
              filename={exportFilename('kind', [{ namespace: null, name: name || 'cluster' }])}
            />
          </div>
        }
        confirmLabel="Create cluster"
        onConfirm={confirmCreate}
        onCancel={() => setConfirmingCreate(false)}
      />
      <ConfirmDialog
        open={deleting !== undefined}
        title="Delete local cluster"
        items={[{ namespace: null, name: deleting ?? '' }]}
        detail={
          <span>
            Deletes the kind cluster and its node containers, and removes its kubeconfig context.
            This cannot be undone.
          </span>
        }
        destructive
        confirmLabel="Delete cluster"
        onConfirm={confirmDelete}
        onCancel={() => setDeleting(undefined)}
      />
    </fieldset>
  )
}
