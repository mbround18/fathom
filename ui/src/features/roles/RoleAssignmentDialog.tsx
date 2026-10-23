/**
 * 033 US4: assign a role to an IdP user/group. The backend only renders YAML
 * (`PreviewRoleAssignment`, no cluster mutation); the write is the existing edit-session
 * machinery: `YamlDiffView` -> `ConfirmDialog` -> `ApplyResourceYaml`, gated by RBAC.
 */
import { useEffect, useRef, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { YamlDiffView } from '@companion/terminal'
import { Button, Input } from '@companion/ui'
import { exportFilename } from '@/features/manifest-export/clean'
import { ManifestActions } from '@/features/manifest-export/ManifestActions'
import { CreateRoleOption } from '@/components/rbac/CreateRoleOption'
import { RoleForm } from '@/components/rbac/RoleForm'
import { Gated } from '@/lib/capabilities'
import type { RoleAssignmentSubject, RoleRefKind } from '@/protocol/types'
import { detailKey, useStore } from '@/store'

import { type DirectorySource, filterGroups, filterUsers, useStoreDirectory } from './directory'

export interface RoleAssignmentDialogProps {
  onClose: () => void
  /** Namespace for a RoleBinding; `null` renders a ClusterRoleBinding. */
  namespace: string | null
  /** Test seam; defaults to the store-backed directory. */
  source?: DirectorySource
  /**
   * 045 FR-030: the issuer this form belongs to. The guided path hands over the issuer it just
   * configured, which is not necessarily the first saved one.
   */
  issuerId?: string
  /**
   * 045 FR-013: a subject chosen elsewhere. The *raw* name: the backend's `render_subject_name`
   * applies the issuer's prefix when it renders the preview, so a pre-prefixed name would be
   * prefixed twice.
   */
  subject?: { kind: 'Group' | 'User'; name: string }
}

export function RoleAssignmentDialog(props: RoleAssignmentDialogProps) {
  const issuers = useStore((s) => s.identity.config.issuers)
  const [issuerId, setIssuerId] = useState<string>(props.issuerId ?? issuers[0]?.id ?? '')
  const storeSource = useStoreDirectory(issuerId)
  const source = props.source ?? storeSource
  return <Body {...props} issuerId={issuerId} setIssuerId={setIssuerId} source={source} />
}

interface BodyProps extends RoleAssignmentDialogProps {
  issuerId: string
  setIssuerId: (id: string) => void
  source: DirectorySource
}

function Body({ onClose, namespace, issuerId, setIssuerId, source, subject: handed }: BodyProps) {
  const issuers = useStore((s) => s.identity.config.issuers)
  const context = useStore((s) => s.session.activeContext)
  const send = useStore((s) => s.send)
  const preview = useStore((s) => s.roles.preview)
  const editSessions = useStore((s) => s.editSessions)
  const openCreateEditSession = useStore((s) => s.openCreateEditSession)
  const reviewEditDiff = useStore((s) => s.reviewEditDiff)
  const applyEditSession = useStore((s) => s.applyEditSession)
  const closeEditSession = useStore((s) => s.closeEditSession)

  const [subjectKind, setSubjectKind] = useState<'Group' | 'User'>(handed?.kind ?? 'Group')
  const [subjectName, setSubjectName] = useState(handed?.name ?? '')
  const [filter, setFilter] = useState('')
  const [roleKind, setRoleKind] = useState<RoleRefKind>('ClusterRole')
  const [roleName, setRoleName] = useState('')
  const [bindingName, setBindingName] = useState('')
  const [confirmOpen, setConfirmOpen] = useState(false)
  // 043 US8: "Create a new role…" swaps in 042's role form and returns here with the new role.
  const [creatingRole, setCreatingRole] = useState(false)

  const kind = namespace ? 'RoleBinding' : 'ClusterRoleBinding'
  const resource = namespace ? 'rolebindings' : 'clusterrolebindings'
  const sessionKey = detailKey(context, kind, namespace, bindingName.trim())
  const session = editSessions[sessionKey]
  // A reply is ours only if it arrives after we asked (ref: no render-time state churn).
  const awaitingSeq = useRef<number | null>(null)

  useEffect(() => {
    const want = awaitingSeq.current
    if (want === null || !preview || preview.seq <= want) return
    awaitingSeq.current = null
    openCreateEditSession(
      {
        apiVersion: 'rbac.authorization.k8s.io/v1',
        kind,
        namespace,
        name: bindingName.trim(),
      },
      preview.yaml,
    )
    reviewEditDiff(sessionKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [preview])

  useEffect(() => () => closeEditSession(sessionKey), [sessionKey, closeEditSession])

  useEffect(() => {
    if (issuerId) send({ type: 'IdpRefreshDirectory', data: { issuer_id: issuerId } })
  }, [issuerId, send])

  const users = filterUsers(source.users, filter)
  const groups = filterGroups(source.groups, filter)
  const canPreview =
    issuerId !== '' && subjectName !== '' && roleName.trim() !== '' && bindingName.trim() !== ''
  const showDiff = session !== undefined && session.status !== 'editing'
  const busy = session?.status === 'applying'
  const blocked = session?.status === 'conflict' || session?.status === 'deleted'
  const applied = session?.status === 'applied'

  const requestPreview = () => {
    const subject: RoleAssignmentSubject =
      subjectKind === 'Group'
        ? { type: 'Group', data: { name: subjectName } }
        : { type: 'User', data: { username: subjectName } }
    awaitingSeq.current = useStore.getState().roles.previewSeq
    send({
      type: 'PreviewRoleAssignment',
      data: {
        draft: {
          issuer_id: issuerId,
          subject,
          role_ref: { kind: roleKind, name: roleName.trim() },
          namespace,
          binding_name: bindingName.trim(),
        },
      },
    })
  }

  return (
    <div data-testid="role-assignment-dialog" className="flex min-h-0 flex-1 flex-col gap-3 p-3">
      {issuers.length === 0 && (
        <p className="text-xs text-muted-foreground">
          Configure an OIDC issuer first: subjects are rendered with its prefixes.
        </p>
      )}
      {handed && (
        <p className="text-xs" data-testid="role-subject-prefilled">
          Subject: <span className="font-mono">{handed.name}</span> ({handed.kind.toLowerCase()}),
          handed over with the issuer <span className="font-mono">{issuerId}</span>. The prefix is
          applied when the binding is rendered below.
        </p>
      )}
      {source.error && (
        <p data-testid="role-directory-error" className="text-xs text-status-warn">
          Directory unavailable{source.stale ? ' (showing cached data)' : ''}: {source.error}
        </p>
      )}

      {showDiff ? (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          {preview && preview.warnings.length > 0 && (
            <ul data-testid="role-preview-warnings" className="text-xs text-status-warn">
              {preview.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
          <p className="text-xs">
            Subject:{' '}
            <span data-testid="role-rendered-subject" className="font-mono">
              {preview?.renderedSubjectName}
            </span>
          </p>
          <YamlDiffView session={session} />
          {/* 040 US3 (FR-018): the full manifest Apply sends. */}
          <ManifestActions
            text={session.editedYaml}
            filename={exportFilename(kind, [{ namespace, name: bindingName.trim() }])}
            what="manifest"
          />
          {(blocked || session.status === 'error') && session.lastError && (
            <p
              data-testid="yaml-apply-error"
              className="font-mono text-xs break-words text-destructive"
            >
              {session.lastError}
            </p>
          )}
          <div className="flex shrink-0 justify-end gap-2">
            <Button size="sm" variant="ghost" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Gated
              apiGroup="rbac.authorization.k8s.io"
              verb="create"
              resource={resource}
              namespace={namespace}
            >
              <Button
                data-testid="yaml-apply"
                size="sm"
                disabled={busy || blocked || applied}
                onClick={() => setConfirmOpen(true)}
              >
                Apply
              </Button>
            </Gated>
          </div>
        </div>
      ) : creatingRole ? (
        <RoleForm
          mode="create"
          kind={roleKind}
          {...(namespace ? { namespace } : {})}
          onClose={() => setCreatingRole(false)}
          onCreated={(role) => {
            setRoleKind(role.roleKind)
            setRoleName(role.name)
            setCreatingRole(false)
          }}
        />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-3 text-xs">
          <label className="flex flex-col gap-1">
            Issuer
            <select
              data-testid="role-issuer"
              className="border border-border bg-background px-2 py-1"
              value={issuerId}
              onChange={(e) => setIssuerId(e.target.value)}
            >
              {issuers.map((i) => (
                <option key={i.id} value={i.id}>
                  {i.id}
                </option>
              ))}
            </select>
          </label>
          <div className="flex gap-2">
            {(['Group', 'User'] as const).map((k) => (
              <Button
                key={k}
                size="sm"
                variant={subjectKind === k ? 'default' : 'outline'}
                data-testid={`role-subject-kind-${k.toLowerCase()}`}
                onClick={() => {
                  setSubjectKind(k)
                  setSubjectName('')
                }}
              >
                {k}
              </Button>
            ))}
          </div>
          <Input
            data-testid="role-subject-filter"
            placeholder={`Filter ${subjectKind === 'Group' ? 'groups' : 'users'}`}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <ul className="max-h-40 overflow-auto border border-border">
            {(subjectKind === 'Group'
              ? groups.map((g) => g.name)
              : users.map((u) => u.username)
            ).map((n) => (
              <li key={n}>
                <button
                  type="button"
                  data-testid={`role-subject-option-${n}`}
                  aria-pressed={subjectName === n}
                  className={`w-full px-2 py-1 text-left font-mono ${subjectName === n ? 'bg-accent' : ''}`}
                  onClick={() => setSubjectName(n)}
                >
                  {n}
                </button>
              </li>
            ))}
          </ul>
          <div className="flex gap-2">
            <select
              data-testid="role-ref-kind"
              className="border border-border bg-background px-2 py-1"
              value={roleKind}
              onChange={(e) => setRoleKind(e.target.value as RoleRefKind)}
            >
              <option value="ClusterRole">ClusterRole</option>
              {namespace && <option value="Role">Role</option>}
            </select>
            <Input
              data-testid="role-ref-name"
              placeholder="Role name (e.g. edit)"
              value={roleName}
              onChange={(e) => setRoleName(e.target.value)}
            />
          </div>
          <CreateRoleOption
            kind={roleKind}
            namespace={namespace}
            testId="role-assignment-create-role"
            onSelect={() => setCreatingRole(true)}
          />
          <Input
            data-testid="role-binding-name"
            placeholder="Binding name"
            value={bindingName}
            onChange={(e) => setBindingName(e.target.value)}
          />
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              data-testid="role-preview"
              size="sm"
              disabled={!canPreview}
              onClick={requestPreview}
            >
              Preview
            </Button>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={confirmOpen}
        title={`Create ${bindingName || kind}`}
        items={[{ namespace, name: bindingName }]}
        detail="Creates the binding as described above."
        confirmLabel="Create"
        onConfirm={() => {
          setConfirmOpen(false)
          void applyEditSession(sessionKey)
        }}
        onCancel={() => setConfirmOpen(false)}
      />

      {applied && (
        <p data-testid="role-assignment-applied" className="text-xs text-status-ok">
          Created.
        </p>
      )}
    </div>
  )
}
