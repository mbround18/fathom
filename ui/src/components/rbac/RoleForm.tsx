/**
 * Create/edit a Role or ClusterRole (042-role-create).
 *
 * Mirrors `RoleBindingForm.tsx`: structured fields generate the manifest, and the existing edit
 * session machinery (`ui/src/store/editSession.ts`) carries it through review → confirm → apply
 * (`CreateResourceYaml` for create, `ApplyResourceYaml` with the live `resourceVersion` for edit),
 * so the write lands in the mutation audit log (026) like every other one.
 *
 * - Review is always available (the manifest can be copied or downloaded without write access);
 *   Apply is disabled with the API server's reason when the self-access review denies the write
 *   (018), and wrapped in `Gated` like every mutating control.
 * - Edit mode patches only `rules` into the live object's `raw_json`, so the diff shows only the
 *   operator's edit. It warns on `system:` roles (Kubernetes' own, often reconciled back on API
 *   server start) and on aggregated ClusterRoles (the controller rewrites their rules).
 * - The API server's escalation-prevention error ("attempting to grant RBAC permissions not
 *   currently held") is explained above the verbatim message.
 * - After a create, `onBind` offers "Bind this role…" (the launcher swaps in `RoleBindingForm`).
 */
import { useEffect, useMemo, useState } from 'react'

import { Button, Input } from '@companion/ui'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { YamlDiffView } from '@/components/panels/YamlDiffView'
import { exportFilename } from '@/features/manifest-export/clean'
import { ManifestActions } from '@/features/manifest-export/ManifestActions'
import { RBAC_GROUP, useMutationAccess } from '@/hooks/useMutationAccess'
import { Gated } from '@/lib/capabilities'
import type { ClusterRoleDetails, RoleDetails } from '@/protocol/types'
import { detailKey, useStore } from '@/store'

import { RuleEditor } from './RuleEditor'
import {
  emptyRule,
  escalationMissing,
  isEscalationError,
  patchRoleRules,
  roleManifestYaml,
  roleNameProblem,
  roleWarnings,
  rulesFromRawJson,
  rulesValid,
  type RoleKind,
  type RuleInput,
} from './roleYaml'

/** What "Bind this role…" needs to prefill the binding form. */
export interface BindTarget {
  roleKind: RoleKind
  name: string
  namespace: string | null
}

export interface RoleFormCreateProps {
  mode: 'create'
  kind: RoleKind
  onClose: () => void
  onBind?: (target: BindTarget, bindingKind: 'RoleBinding' | 'ClusterRoleBinding') => void
  /**
   * 043 US8 (FR-041): a flow that opened this form to pick a new role (the binding form's or
   * Assign role dialog's "Create a new role…") gets the created role back as its `roleRef`.
   */
  onCreated?: (target: BindTarget) => void
  /** Starting namespace for a Role (defaults to the active namespace). */
  namespace?: string
}

export interface RoleFormEditProps {
  mode: 'edit'
  kind: RoleKind
  existing: RoleDetails | ClusterRoleDetails
  onClose: () => void
}

export type RoleFormProps = RoleFormCreateProps | RoleFormEditProps

function sectionFor(kind: RoleKind): string {
  return kind === 'Role' ? 'access/roles' : 'access/clusterroles'
}

function existingNamespace(existing: RoleDetails | ClusterRoleDetails): string | null {
  return 'namespace' in existing ? existing.namespace : null
}

export function RoleForm(props: RoleFormProps) {
  const { mode, kind, onClose } = props
  const context = useStore((s) => s.session.activeContext)
  const namespaces = useStore((s) => s.session.namespaces)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const catalog = useStore((s) => s.rbacAccess.apiResources)
  const send = useStore((s) => s.send)
  const openEditSession = useStore((s) => s.openEditSession)
  const openCreateEditSession = useStore((s) => s.openCreateEditSession)
  const updateEditedYaml = useStore((s) => s.updateEditedYaml)
  const reviewEditDiff = useStore((s) => s.reviewEditDiff)
  const applyEditSession = useStore((s) => s.applyEditSession)
  const closeEditSession = useStore((s) => s.closeEditSession)
  const discardEditSession = useStore((s) => s.discardEditSession)
  const editSessions = useStore((s) => s.editSessions)

  const resource = kind === 'Role' ? 'roles' : 'clusterroles'
  const existing = mode === 'edit' ? props.existing : undefined

  const [name, setName] = useState(existing?.name ?? '')
  const initialNamespace = mode === 'create' ? props.namespace : undefined
  const [namespace, setNamespace] = useState<string>(
    existing ? (existingNamespace(existing) ?? '') : initialNamespace || (activeNamespace ?? ''),
  )
  const [rules, setRules] = useState<RuleInput[]>(() =>
    existing ? rulesFromRawJson(existing.raw_json) : [emptyRule('read-only')],
  )
  const [confirmOpen, setConfirmOpen] = useState(false)
  // Create has no live object to diff against, so it opens on the manifest; edit on the diff.
  const [reviewView, setReviewView] = useState<'diff' | 'manifest'>(
    mode === 'create' ? 'manifest' : 'diff',
  )

  const editSessionKey = useMemo(
    () =>
      existing ? detailKey(context, kind, existingNamespace(existing), existing.name) : undefined,
    [existing, context, kind],
  )
  const [createSessionKey, setCreateSessionKey] = useState<string | undefined>(undefined)
  const sessionKey = mode === 'edit' ? editSessionKey : createSessionKey

  const effectiveNamespace = kind === 'Role' ? namespace.trim() || null : null
  const verb = mode === 'create' ? 'create' : 'update'
  const access = useMutationAccess({
    verb,
    resource,
    group: RBAC_GROUP,
    namespace: effectiveNamespace,
  })
  const denied = access.status === 'denied'
  const deniedReason =
    access.reason ??
    `You don't have permission to ${verb} ${resource}${effectiveNamespace ? ` in ${effectiveNamespace}` : ''}.`

  // The resource/API group pickers are fed from discovery; fetch it once per context.
  useEffect(() => {
    if (!catalog) send({ type: 'RefreshApiResources' })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context])

  // Edit mode: seed the edit session from the live object's raw_json once.
  useEffect(() => {
    if (!existing || !editSessionKey) return
    openEditSession(
      {
        section: sectionFor(kind),
        kind,
        namespace: existingNamespace(existing),
        name: existing.name,
      },
      existing.raw_json,
    )
    return () => closeEditSession(editSessionKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode])

  // Edit mode: regenerate the edit buffer whenever the rules change.
  useEffect(() => {
    if (!existing || !sessionKey) return
    updateEditedYaml(sessionKey, patchRoleRules(existing.raw_json, kind, rules))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionKey, rules])

  const session = sessionKey ? editSessions[sessionKey] : undefined
  const busy = session?.status === 'applying'
  const blocked = session?.status === 'conflict' || session?.status === 'deleted'
  const applied = session?.status === 'applied'

  // RBAC lists are fetched, not watched: after a successful write, refresh the list (so the new
  // row / rule count shows) and, for an edit, the open inspector's details (fresh raw_json and
  // resourceVersion for the next edit).
  useEffect(() => {
    if (!applied) return
    send({ type: kind === 'Role' ? 'RefreshRoles' : 'RefreshClusterRoles' })
    if (mode === 'create' && props.onCreated) {
      props.onCreated({ roleKind: kind, name, namespace: effectiveNamespace })
    }
    if (existing) {
      send(
        kind === 'Role'
          ? {
              type: 'SelectRole',
              data: { namespace: existingNamespace(existing) ?? '', name: existing.name },
            }
          : { type: 'SelectClusterRole', data: { name: existing.name } },
      )
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied])

  const showReview =
    session !== undefined &&
    (session.status === 'diffing' ||
      session.status === 'applying' ||
      session.status === 'conflict' ||
      session.status === 'deleted' ||
      session.status === 'error' ||
      session.status === 'applied')

  const nameProblem = mode === 'create' ? roleNameProblem(name) : null
  const warnings = roleWarnings(name, existing?.raw_json)
  const canReview =
    nameProblem === null &&
    (kind === 'ClusterRole' || namespace.trim().length > 0) &&
    rulesValid(rules, kind)

  const review = () => {
    if (mode === 'create') {
      const yaml = roleManifestYaml({ kind, name, namespace: effectiveNamespace, rules })
      const key = openCreateEditSession(
        { apiVersion: 'rbac.authorization.k8s.io/v1', kind, namespace: effectiveNamespace, name },
        yaml,
      )
      setCreateSessionKey(key)
      reviewEditDiff(key)
    } else if (sessionKey) {
      reviewEditDiff(sessionKey)
    }
  }

  const backToEdit = () => {
    if (mode === 'create') {
      if (createSessionKey) closeEditSession(createSessionKey)
      setCreateSessionKey(undefined)
    } else if (sessionKey && existing) {
      // Back to `editing`, then re-apply the operator's rules on top of the live object.
      discardEditSession(sessionKey)
      updateEditedYaml(sessionKey, patchRoleRules(existing.raw_json, kind, rules))
    }
  }

  const escalation = session?.status === 'error' && isEscalationError(session.lastError)
  const missing = escalation && session.lastError ? escalationMissing(session.lastError) : []
  const bindTarget: BindTarget = { roleKind: kind, name, namespace: effectiveNamespace }
  const onBind = mode === 'create' ? props.onBind : undefined

  return (
    <div
      data-testid="rbac-role-form"
      className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-3"
    >
      {warnings.system && (
        <p
          data-testid="rbac-role-warning-system"
          className="border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
        >
          <code>system:</code> roles belong to Kubernetes. The API server reconciles many of them on
          start, and changing them can break cluster components.
        </p>
      )}
      {warnings.aggregated && (
        <p
          data-testid="rbac-role-warning-aggregated"
          className="border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
        >
          This ClusterRole has an <code>aggregationRule</code>. The aggregation controller
          overwrites its rules from the matching ClusterRoles, so edits here will not stick: edit
          (or label) the aggregated ClusterRoles instead.
        </p>
      )}

      {session && showReview ? (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <div role="group" aria-label="Review view" className="flex shrink-0 gap-1">
            {(['diff', 'manifest'] as const).map((v) => (
              <Button
                key={v}
                size="sm"
                variant={reviewView === v ? 'secondary' : 'ghost'}
                aria-pressed={reviewView === v}
                data-testid={`rbac-role-view-${v}`}
                onClick={() => setReviewView(v)}
              >
                {v === 'diff' ? 'Diff' : 'Manifest'}
              </Button>
            ))}
          </div>
          {reviewView === 'diff' ? (
            <YamlDiffView session={session} />
          ) : (
            <pre
              data-testid="rbac-role-manifest"
              // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- scrollable region must be keyboard reachable
              tabIndex={0}
              className="min-h-0 flex-1 overflow-auto border border-border bg-background p-2 font-mono text-xs leading-relaxed whitespace-pre"
            >
              {session.editedYaml}
            </pre>
          )}
          <ManifestActions
            text={session.editedYaml}
            filename={exportFilename(kind, [{ namespace: effectiveNamespace, name }])}
            what="manifest"
          />
          {escalation && (
            <div
              data-testid="rbac-role-escalation"
              className="flex flex-col gap-1 border border-destructive/50 bg-destructive/10 px-2 py-1.5 text-xs text-destructive"
            >
              <p className="font-medium">
                The API server refused: this role grants permissions you do not hold yourself.
              </p>
              <p>
                Kubernetes blocks privilege escalation. Remove the rules you do not hold, or ask an
                administrator for the <code>escalate</code> verb on {resource}.
              </p>
              {missing.length > 0 && (
                <ul className="list-disc pl-4 font-mono">
                  {missing.map((m) => (
                    <li key={m}>{m}</li>
                  ))}
                </ul>
              )}
            </div>
          )}
          {blocked || session.status === 'error' ? (
            <div className="flex flex-col gap-1 text-xs text-destructive">
              {session.lastError && (
                <p data-testid="yaml-apply-error" className="font-mono break-words">
                  {session.lastError}
                </p>
              )}
              {blocked && (
                <Button
                  data-testid="yaml-reload"
                  size="sm"
                  variant="outline"
                  className="w-fit"
                  onClick={onClose}
                >
                  Reload latest
                </Button>
              )}
            </div>
          ) : null}
          {denied && !applied && (
            <p
              data-testid="rbac-role-apply-disabled"
              className="border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
            >
              {deniedReason} You can still copy or download the manifest.
            </p>
          )}
          {applied ? (
            <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
              <p data-testid="rbac-role-applied" className="mr-auto text-xs text-status-ok">
                {mode === 'create' ? `Created ${kind} ${name}.` : 'Applied.'}
              </p>
              {onBind && (
                <>
                  <Button
                    data-testid="rbac-role-bind-open"
                    size="sm"
                    onClick={() =>
                      onBind(bindTarget, kind === 'Role' ? 'RoleBinding' : 'ClusterRoleBinding')
                    }
                  >
                    Bind this role…
                  </Button>
                  {kind === 'ClusterRole' && (
                    <Button
                      data-testid="rbac-role-bind-namespaced-open"
                      size="sm"
                      variant="outline"
                      onClick={() => onBind(bindTarget, 'RoleBinding')}
                    >
                      Bind in a namespace…
                    </Button>
                  )}
                </>
              )}
              <Button size="sm" variant="ghost" onClick={onClose}>
                Close
              </Button>
            </div>
          ) : (
            <div className="flex shrink-0 items-center justify-end gap-2">
              <Button
                size="sm"
                variant="ghost"
                data-testid="rbac-role-back"
                disabled={busy}
                onClick={backToEdit}
              >
                Back
              </Button>
              <Button size="sm" variant="ghost" disabled={busy} onClick={onClose}>
                Cancel
              </Button>
              <Gated
                apiGroup={RBAC_GROUP}
                verb={verb}
                resource={resource}
                namespace={effectiveNamespace}
              >
                <Button
                  data-testid="yaml-apply"
                  size="sm"
                  title={denied ? deniedReason : undefined}
                  disabled={busy || blocked || denied || !sessionKey}
                  onClick={() => setConfirmOpen(true)}
                >
                  {mode === 'create' ? 'Create' : 'Apply'}
                </Button>
              </Gated>
            </div>
          )}
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <label htmlFor="rbac-role-name-input" className="flex flex-col gap-1 text-xs">
            Name
            <Input
              id="rbac-role-name-input"
              data-testid="rbac-role-name"
              value={name}
              disabled={mode === 'edit'}
              onChange={(e) => setName(e.target.value)}
            />
            {nameProblem && name.length > 0 && (
              <span data-testid="rbac-role-name-error" className="text-destructive">
                {nameProblem}
              </span>
            )}
          </label>

          {kind === 'Role' && (
            <label htmlFor="rbac-role-namespace-input" className="flex flex-col gap-1 text-xs">
              Namespace
              <Input
                id="rbac-role-namespace-input"
                data-testid="rbac-role-namespace"
                list="rbac-role-namespaces"
                value={namespace}
                disabled={mode === 'edit'}
                onChange={(e) => setNamespace(e.target.value)}
              />
              <datalist id="rbac-role-namespaces">
                {namespaces.map((ns) => (
                  <option key={ns} value={ns} />
                ))}
              </datalist>
            </label>
          )}

          <RuleEditor kind={kind} rules={rules} onChange={setRules} catalog={catalog} />

          {denied && (
            <p
              data-testid="rbac-role-apply-disabled"
              className="border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
            >
              {deniedReason} You can still review, copy or download the manifest.
            </p>
          )}

          <div className="flex shrink-0 items-center justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button data-testid="rbac-role-review" size="sm" disabled={!canReview} onClick={review}>
              Review
            </Button>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={confirmOpen}
        title={`${mode === 'create' ? 'Create' : 'Apply changes to'} ${kind} ${name}`}
        items={[{ namespace: effectiveNamespace, name }]}
        detail={
          mode === 'create'
            ? `Creates the ${kind} with the rules shown above.`
            : `The edited rules replace the live ${kind}'s rules. Anyone bound to it gains or loses access immediately.`
        }
        confirmLabel={mode === 'create' ? 'Create' : 'Apply'}
        onConfirm={() => {
          setConfirmOpen(false)
          if (sessionKey) void applyEditSession(sessionKey)
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  )
}
