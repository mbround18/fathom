/**
 * Standalone "look up an identity's effective namespace access" view (011-rbac-oidc-administration
 * US4, T040). Not a `ResourceDescriptor` — there is no list/detail split and no backend
 * list-refresh, so like `PortForwardsPanel` this is rendered directly for its section rather than
 * going through the generic descriptor table (the `PaneHost` special-case branch is wired by a
 * separate integration pass, per tasks.md T041/T042).
 *
 * Dispatches only the read-only `ResolveNamespaceAccess` command (FR-011/FR-012) and renders the
 * `NamespaceAccessResolved` reply from `rbacAccess.namespaceAccess`. Never fires a write/mutating
 * command.
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { RoleCreateLauncher } from '@/components/rbac/RoleCreateLauncher'
import type { NamespaceAccessEntry, NamespaceAccessResolved } from '@/protocol/types'
import { useStore } from '@/store'

// `DegradedScopeBanner` (011, built in parallel by another process) did not exist in
// `@/components/rbac/` yet when this file was written, so this is a minimal inline stand-in with
// the same test hook (`rbac-degraded-scope`) and a similar props shape. Swap in the real import
// once `ui/src/components/rbac/DegradedScopeBanner.tsx` lands (integration will reconcile).
function DegradedScopeBanner({ scopes, message }: { scopes: readonly string[]; message?: string }) {
  return (
    <div
      data-testid="rbac-degraded-scope"
      className="border border-amber-600/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400"
    >
      <p className="font-medium">
        {message ?? 'Some scopes could not be listed with the current identity:'}
      </p>
      <ul className="mt-1 list-disc pl-4">
        {scopes.map((scope) => (
          <li key={scope}>{scope}</li>
        ))}
      </ul>
    </div>
  )
}

type SubjectKind = 'User' | 'Group' | 'ServiceAccount'

interface SubmittedSubject {
  subject_kind: string
  subject_name: string
  subject_namespace: string | null
}

function sameSubject(a: SubmittedSubject, b: NamespaceAccessResolved): boolean {
  return (
    a.subject_kind === b.subject_kind &&
    a.subject_name === b.subject_name &&
    a.subject_namespace === b.subject_namespace
  )
}

function EntryTable({
  entries,
  showNamespace,
}: {
  entries: NamespaceAccessEntry[]
  showNamespace: boolean
}) {
  return (
    <table className="w-full border-collapse text-xs">
      <thead>
        <tr className="border-b border-border text-left text-muted-foreground">
          {showNamespace && <th className="py-1 pr-3 font-medium">Namespace</th>}
          <th className="py-1 pr-3 font-medium">Verbs</th>
          <th className="py-1 pr-3 font-medium">Resources</th>
          <th className="py-1 pr-3 font-medium">Via</th>
        </tr>
      </thead>
      <tbody>
        {entries.map((entry, idx) => (
          <tr key={`${entry.namespace}-${idx}`} className="border-b border-border/50 align-top">
            {showNamespace && <td className="py-1 pr-3 font-mono">{entry.namespace}</td>}
            <td className="py-1 pr-3">{entry.verbs.join(', ')}</td>
            <td className="py-1 pr-3">{entry.resources.join(', ')}</td>
            <td className="py-1 pr-3 text-muted-foreground">{entry.via.join(', ')}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

export function NamespaceAccessView() {
  const request = useStore((s) => s.request)
  const result = useStore((s) => s.rbacAccess.namespaceAccess)
  const activeNamespace = useStore((s) => s.session.activeNamespace)

  const [subjectKind, setSubjectKind] = useState<SubjectKind>('User')
  const [subjectName, setSubjectName] = useState('')
  const [subjectNamespace, setSubjectNamespace] = useState('')
  const [error, setError] = useState<string | undefined>(undefined)
  const [isLoading, setIsLoading] = useState(false)
  const [submitted, setSubmitted] = useState<SubmittedSubject | undefined>(undefined)

  const onSubmit = (e: React.SubmitEvent<HTMLFormElement>) => {
    e.preventDefault()
    const trimmedName = subjectName.trim()
    if (trimmedName === '') {
      setError('Subject name is required.')
      return
    }
    setError(undefined)
    const namespace = subjectNamespace.trim() === '' ? null : subjectNamespace.trim()
    const data = {
      subject_kind: subjectKind,
      subject_name: trimmedName,
      subject_namespace: namespace,
    }
    setSubmitted(data)
    setIsLoading(true)
    void request({ type: 'ResolveNamespaceAccess', data }).finally(() => {
      setIsLoading(false)
    })
  }

  const showing = submitted && result && sameSubject(submitted, result) ? result : undefined

  return (
    <div
      data-testid="rbac-namespace-access-lookup"
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
    >
      <form onSubmit={onSubmit} className="flex flex-wrap items-end gap-3">
        <label htmlFor="rbac-namespace-access-kind" className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Subject kind</span>
          <select
            id="rbac-namespace-access-kind"
            data-testid="rbac-namespace-access-kind"
            className="h-8 rounded-none border border-input bg-transparent px-2 text-xs"
            value={subjectKind}
            onChange={(e) => {
              setSubjectKind(e.target.value as SubjectKind)
            }}
          >
            <option value="User">User</option>
            <option value="Group">Group</option>
            <option value="ServiceAccount">ServiceAccount</option>
          </select>
        </label>

        <label htmlFor="rbac-namespace-access-name" className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Subject name</span>
          <Input
            id="rbac-namespace-access-name"
            data-testid="rbac-namespace-access-name"
            value={subjectName}
            onChange={(e) => {
              setSubjectName(e.target.value)
            }}
            placeholder="e.g. jane@example.com"
          />
        </label>

        <label
          htmlFor="rbac-namespace-access-subject-namespace"
          className="flex flex-col gap-1 text-xs"
        >
          <span className="text-muted-foreground">Subject namespace (ServiceAccount only)</span>
          <Input
            id="rbac-namespace-access-subject-namespace"
            data-testid="rbac-namespace-access-subject-namespace"
            value={subjectNamespace}
            onChange={(e) => {
              setSubjectNamespace(e.target.value)
            }}
            placeholder="optional"
          />
        </label>

        <Button
          type="submit"
          size="sm"
          disabled={isLoading}
          data-testid="rbac-namespace-access-submit"
        >
          {isLoading ? 'Looking up…' : 'Look up'}
        </Button>
      </form>

      {error && (
        <p
          role="alert"
          data-testid="rbac-namespace-access-error"
          className="text-xs text-destructive"
        >
          {error}
        </p>
      )}

      <p className="text-xs text-muted-foreground">
        User and Group names are opaque strings here — this lookup does not validate them against
        any identity provider, so a typo or a non-existent identity resolves the same as a real one
        with no bindings.
      </p>

      {showing && (
        <div className="flex flex-col gap-4">
          {showing.degraded_scopes.length > 0 && (
            <DegradedScopeBanner
              scopes={showing.degraded_scopes}
              message="Some namespaces/scopes could not be listed with the operator's current identity; results below may be incomplete:"
            />
          )}

          {showing.dangling_bindings.length > 0 && (
            <div
              data-testid="rbac-namespace-access-dangling"
              className="border border-amber-600/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400"
            >
              {showing.dangling_bindings.length} binding(s) reference a Role/ClusterRole that no
              longer exists: {showing.dangling_bindings.join(', ')}
            </div>
          )}

          {!showing.found_any ? (
            <p data-testid="rbac-namespace-access-empty" className="text-xs text-muted-foreground">
              No RoleBindings or ClusterRoleBindings grant this subject any access.
            </p>
          ) : (
            <>
              {showing.cluster_wide.length > 0 && (
                <section
                  data-testid="rbac-namespace-access-clusterwide"
                  className="flex flex-col gap-2"
                >
                  <h3 className="text-xs font-semibold text-muted-foreground uppercase">
                    Cluster-wide
                  </h3>
                  <EntryTable entries={showing.cluster_wide} showNamespace={false} />
                </section>
              )}

              {showing.per_namespace.length > 0 && (
                <section
                  data-testid="rbac-namespace-access-per-namespace"
                  className="flex flex-col gap-2"
                >
                  <h3 className="text-xs font-semibold text-muted-foreground uppercase">
                    Per-namespace
                  </h3>
                  <EntryTable entries={showing.per_namespace} showNamespace={true} />
                </section>
              )}
            </>
          )}

          {/* 043 US8: the access this identity lacks starts with a role (042's form). */}
          <p
            data-testid="rbac-namespace-access-create-role"
            className="flex flex-wrap items-center gap-x-1 text-xs text-muted-foreground"
          >
            Not the access you want?
            <RoleCreateLauncher
              kind="Role"
              label="Create a role…"
              testId="rbac-namespace-access-create-role-open"
              variant="link"
              namespace={showing.subject_namespace ?? activeNamespace}
            />
            Then bind it to this {showing.subject_kind.toLowerCase()}.
          </p>
        </div>
      )}
    </div>
  )
}
