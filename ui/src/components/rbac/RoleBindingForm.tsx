/**
 * Create/edit a RoleBinding or ClusterRoleBinding (011-rbac-oidc-administration US2 T027-T029,
 * US3 T031-T032, T034).
 *
 * Both modes reuse `ui/src/store/editSession.ts`'s existing edit → review-diff → confirm → apply
 * machinery (T032: no new state machine) and the exact `YamlDiffView`/`ConfirmDialog` test hooks
 * (`yaml-diff`, `yaml-apply`, `confirm-submit`, `yaml-reload`) `YamlEditTab.tsx` already
 * established for US1, per the task's explicit "reuse these testids" instruction.
 *
 * Edit mode design choice (US3 T031, documented per the task's request): structured fields that
 * regenerate the edit session's YAML buffer on every change (`patchRoleBindingSubjects`), rather
 * than reusing `YamlEditTab`'s free-text textarea wholesale. This keeps the operator editing via
 * the same Role/ClusterRole + Subject controls used for create, while `openEditSession` still
 * seeds the session from the live object's `raw_json` so `expected_resource_version` is set from
 * its real `resourceVersion` and only `roleRef`/`subjects` change — every other field (labels,
 * annotations, ownerReferences, ...) is carried through untouched, so the diff view shows only
 * the operator's actual edit.
 */
import { useEffect, useMemo, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ConfirmDialog } from '@/components/ConfirmDialog'
import { YamlDiffView } from '@/components/panels/YamlDiffView'
import type {
  ClusterRoleBindingDetails,
  ClusterRoleSummary,
  RbacAccessResult,
  RoleBindingDetails,
  RoleSummary,
  SubjectSummary,
} from '@/protocol/types'
import { detailKey, listKey, useStore } from '@/store'

import { SubjectEditor, subjectsValid } from './SubjectEditor'
import { patchRoleBindingSubjects, roleBindingManifestYaml } from './roleBindingYaml'

export type RoleBindingKind = 'RoleBinding' | 'ClusterRoleBinding'
type RoleRefKind = 'Role' | 'ClusterRole'

export interface RoleBindingFormCreateProps {
  mode: 'create'
  kind: RoleBindingKind
  onClose: () => void
}

export interface RoleBindingFormEditProps {
  mode: 'edit'
  kind: RoleBindingKind
  existing: RoleBindingDetails | ClusterRoleBindingDetails
  onClose: () => void
}

export type RoleBindingFormProps = RoleBindingFormCreateProps | RoleBindingFormEditProps

function sectionFor(kind: RoleBindingKind): string {
  return kind === 'RoleBinding' ? 'access/rolebindings' : 'access/clusterrolebindings'
}

function existingNamespace(
  existing: RoleBindingDetails | ClusterRoleBindingDetails,
): string | null {
  return 'namespace' in existing ? existing.namespace : null
}

function parseRoleRef(roleRef: string): { kind: RoleRefKind; name: string } {
  const [kind, ...rest] = roleRef.split('/')
  const name = rest.join('/')
  return { kind: kind === 'ClusterRole' ? 'ClusterRole' : 'Role', name }
}

/** Finds the reply matching this check among the latest `RbacSelfAccessUpdated` results. */
function findAccess(
  results: RbacAccessResult[] | undefined,
  verb: string,
  resource: string,
  namespace: string | null,
): RbacAccessResult | undefined {
  return results?.find(
    (r) => r.check.verb === verb && r.check.resource === resource && r.check.namespace === namespace,
  )
}

export function RoleBindingForm(props: RoleBindingFormProps) {
  const { mode, kind, onClose } = props
  const context = useStore((s) => s.session.activeContext)
  const namespaces = useStore((s) => s.session.namespaces)
  const send = useStore((s) => s.send)
  const lists = useStore((s) => s.lists)
  const selfAccess = useStore((s) => s.rbacAccess.selfAccess)
  const openEditSession = useStore((s) => s.openEditSession)
  const openCreateEditSession = useStore((s) => s.openCreateEditSession)
  const updateEditedYaml = useStore((s) => s.updateEditedYaml)
  const reviewEditDiff = useStore((s) => s.reviewEditDiff)
  const applyEditSession = useStore((s) => s.applyEditSession)
  const closeEditSession = useStore((s) => s.closeEditSession)
  const editSessions = useStore((s) => s.editSessions)

  const resource = kind === 'RoleBinding' ? 'rolebindings' : 'clusterrolebindings'
  const existing = mode === 'edit' ? props.existing : undefined

  const [name, setName] = useState(existing?.name ?? '')
  const [namespace, setNamespace] = useState<string>(
    existing ? (existingNamespace(existing) ?? '') : '',
  )
  const [roleRefKind, setRoleRefKind] = useState<RoleRefKind>(
    existing ? parseRoleRef(existing.role_ref).kind : kind === 'ClusterRoleBinding' ? 'ClusterRole' : 'Role',
  )
  const [roleRefName, setRoleRefName] = useState<string>(
    existing ? parseRoleRef(existing.role_ref).name : '',
  )
  const [subjects, setSubjects] = useState<SubjectSummary[]>(existing?.subjects ?? [])
  const [confirmOpen, setConfirmOpen] = useState(false)
  // Edit mode's session key is derived, not stored: it is fully determined by `existing` (stable
  // for the life of this form instance), so deriving it avoids a `setState` call inside the
  // seeding effect below (react-hooks/set-state-in-effect). Create mode's key only exists once
  // `openCreateEditSession` is called from `review()`, so it stays local state.
  const editSessionKey = useMemo(
    () => (existing ? detailKey(context, kind, existingNamespace(existing), existing.name) : undefined),
    [existing, context, kind],
  )
  const [createSessionKey, setCreateSessionKey] = useState<string | undefined>(undefined)
  const sessionKey = mode === 'edit' ? editSessionKey : createSessionKey

  const effectiveNamespace = kind === 'RoleBinding' ? namespace || null : null
  const verb = mode === 'create' ? 'create' : 'update'
  const access = findAccess(selfAccess, verb, resource, effectiveNamespace)
  const allowed = access?.allowed === true

  // T028/T034: dispatch (and re-dispatch on namespace change) the self-access-review check.
  useEffect(() => {
    send({
      type: 'RefreshRbacSelfAccess',
      data: { checks: [{ verb, resource, namespace: effectiveNamespace }] },
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [verb, resource, effectiveNamespace])

  // Create mode: fetch the Role/ClusterRole pickers' data once.
  useEffect(() => {
    if (mode !== 'create') return
    const rolesKey = listKey(context, 'access/roles', 'Namespaced', null)
    const clusterRolesKey = listKey(context, 'access/clusterroles', 'Cluster', null)
    if (kind === 'RoleBinding' && !lists[rolesKey]) send({ type: 'RefreshRoles' })
    if (!lists[clusterRolesKey]) send({ type: 'RefreshClusterRoles' })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, kind, context])

  // Edit mode: seed the edit session from the live object's raw_json once.
  useEffect(() => {
    if (!existing || !editSessionKey) return
    const ns = existingNamespace(existing)
    openEditSession({ section: sectionFor(kind), kind, namespace: ns, name: existing.name }, existing.raw_json)
    return () => closeEditSession(editSessionKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode])

  // Edit mode: regenerate the edit buffer whenever roleRef/subjects change.
  useEffect(() => {
    if (!existing || !sessionKey) return
    const patched = patchRoleBindingSubjects(existing.raw_json, roleRefKind, roleRefName, subjects)
    updateEditedYaml(sessionKey, patched)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionKey, roleRefKind, roleRefName, subjects])

  const rolesList = useStore(
    (s) => s.lists[listKey(context, 'access/roles', 'Namespaced', null)],
  )
  const clusterRolesList = useStore(
    (s) => s.lists[listKey(context, 'access/clusterroles', 'Cluster', null)],
  )
  const roleOptions = useMemo<string[]>(() => {
    if (roleRefKind === 'ClusterRole') {
      return ((clusterRolesList?.rows ?? []) as ClusterRoleSummary[]).map((r) => r.name)
    }
    return ((rolesList?.rows ?? []) as RoleSummary[])
      .filter((r) => !namespace || r.namespace === namespace)
      .map((r) => r.name)
  }, [roleRefKind, rolesList, clusterRolesList, namespace])

  const session = sessionKey ? editSessions[sessionKey] : undefined
  const busy = session?.status === 'applying'
  const blocked = session?.status === 'conflict' || session?.status === 'deleted'
  const applied = session?.status === 'applied'
  // Once reviewed, stay on the diff/apply view through every later status (applying, a failed
  // apply's conflict/deleted/error, and applied) rather than only while `status === 'diffing'` —
  // otherwise the apply error/`yaml-reload` affordance would never render (US3 T032/T034).
  const showDiff =
    session !== undefined &&
    (session.status === 'diffing' ||
      session.status === 'applying' ||
      session.status === 'conflict' ||
      session.status === 'deleted' ||
      session.status === 'error' ||
      session.status === 'applied')

  const validSubjects = subjectsValid(subjects)
  const canReview =
    allowed &&
    name.trim().length > 0 &&
    roleRefName.trim().length > 0 &&
    (kind === 'ClusterRoleBinding' || namespace.trim().length > 0) &&
    validSubjects

  const review = () => {
    if (mode === 'create') {
      const yaml = roleBindingManifestYaml({
        kind,
        name,
        namespace: effectiveNamespace,
        roleRefKind,
        roleRefName,
        subjects,
      })
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

  const reload = () => {
    // Edit mode's reload affordance: the operator must re-open the object to pick up its
    // current raw_json/resourceVersion (mirrors YamlEditTab's `yaml-reload`, T032).
    onClose()
  }

  return (
    <div data-testid="rbac-binding-form" className="flex min-h-0 flex-1 flex-col gap-3 p-3">
      {!allowed && (
        <p
          data-testid="rbac-binding-create-disabled"
          className="border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
        >
          {access?.reason ?? `You don't have permission to ${verb} ${resource}${effectiveNamespace ? ` in ${effectiveNamespace}` : ''}.`}
        </p>
      )}

      {session && showDiff ? (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <YamlDiffView session={session} />
          {blocked || session.status === 'error' ? (
            <div className="flex flex-col gap-1 text-xs text-destructive">
              {session.lastError && (
                <p data-testid="yaml-apply-error" className="font-mono break-words">
                  {session.lastError}
                </p>
              )}
              {blocked && (
                <Button data-testid="yaml-reload" size="sm" variant="outline" className="w-fit" onClick={reload}>
                  Reload latest
                </Button>
              )}
            </div>
          ) : null}
          <div className="flex shrink-0 items-center justify-end gap-2">
            <Button size="sm" variant="ghost" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button
              data-testid="yaml-apply"
              size="sm"
              disabled={busy || blocked || applied || !sessionKey}
              onClick={() => setConfirmOpen(true)}
            >
              Apply
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-3">
          <label htmlFor="rbac-binding-name-input" className="flex flex-col gap-1 text-xs">
            Name
            <Input
              id="rbac-binding-name-input"
              data-testid="rbac-binding-name"
              value={name}
              disabled={mode === 'edit'}
              onChange={(e) => setName(e.target.value)}
            />
          </label>

          {kind === 'RoleBinding' && (
            <label htmlFor="rbac-binding-namespace-input" className="flex flex-col gap-1 text-xs">
              Namespace
              <Input
                id="rbac-binding-namespace-input"
                data-testid="rbac-binding-namespace"
                list="rbac-binding-namespaces"
                value={namespace}
                disabled={mode === 'edit'}
                onChange={(e) => setNamespace(e.target.value)}
              />
              <datalist id="rbac-binding-namespaces">
                {namespaces.map((ns) => (
                  <option key={ns} value={ns} />
                ))}
              </datalist>
            </label>
          )}

          <div className="flex flex-wrap items-end gap-2 text-xs">
            {kind === 'RoleBinding' && (
              <label htmlFor="rbac-binding-roleref-kind-select" className="flex flex-col gap-1">
                Role kind
                <select
                  id="rbac-binding-roleref-kind-select"
                  data-testid="rbac-binding-roleref-kind"
                  className="h-8 rounded-none border border-input bg-transparent px-2.5 py-1 text-xs"
                  value={roleRefKind}
                  onChange={(e) => setRoleRefKind(e.target.value === 'ClusterRole' ? 'ClusterRole' : 'Role')}
                >
                  <option value="Role">Role</option>
                  <option value="ClusterRole">ClusterRole</option>
                </select>
              </label>
            )}
            <label htmlFor="rbac-binding-roleref-name-input" className="flex flex-col gap-1">
              {roleRefKind}
              <Input
                id="rbac-binding-roleref-name-input"
                data-testid="rbac-binding-roleref-name"
                list="rbac-binding-roleref-options"
                value={roleRefName}
                onChange={(e) => setRoleRefName(e.target.value)}
              />
              <datalist id="rbac-binding-roleref-options">
                {roleOptions.map((r) => (
                  <option key={r} value={r} />
                ))}
              </datalist>
            </label>
          </div>

          <SubjectEditor subjects={subjects} onChange={setSubjects} />

          <div className="flex shrink-0 items-center justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button data-testid="rbac-binding-review" size="sm" disabled={!canReview} onClick={review}>
              Review
            </Button>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={confirmOpen}
        title={`${mode === 'create' ? 'Create' : 'Apply changes to'} ${name || kind}`}
        items={[{ namespace: effectiveNamespace, name }]}
        detail={
          mode === 'create'
            ? 'Creates the binding as described above.'
            : 'The edited RoleBinding/ClusterRoleBinding replaces the live object. This cannot be undone.'
        }
        confirmLabel={mode === 'create' ? 'Create' : 'Apply'}
        onConfirm={() => {
          setConfirmOpen(false)
          if (sessionKey) void applyEditSession(sessionKey)
        }}
        onCancel={() => setConfirmOpen(false)}
      />

      {applied && (
        <p data-testid="rbac-binding-applied" className="text-xs text-status-ok">
          {mode === 'create' ? 'Created.' : 'Applied.'}
        </p>
      )}
    </div>
  )
}
