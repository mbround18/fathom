/**
 * "New Role"/"New ClusterRole" entry point (042-role-create), mounted in the Roles and
 * ClusterRoles list toolbars. Opens `RoleForm` (create) in a `Sheet`; after a successful create
 * the form's "Bind this role…" swaps the sheet to `RoleBindingForm` prefilled with the new role.
 *
 * 043 US8 reuses it wherever role access is managed: the RoleBindings/ClusterRoleBindings
 * toolbars, the Namespace Access view (as a "Create a role…" link in that namespace) and the
 * command palette (`acceptRequests`, see `roleCreateRequest.ts`). `onCreated` hands the new role
 * back to a caller instead of offering "Bind this role…".
 */
import { useEffect, useState } from 'react'

import { Button, Sheet, SheetContent, SheetHeader, SheetTitle } from '@companion/ui'
import { Gated, useCapability } from '@/lib/capabilities'
import { useStore } from '@/store'

import { RoleBindingForm, type RoleBindingKind } from './RoleBindingForm'
import { RoleForm, type BindTarget } from './RoleForm'
import { takeRoleCreateRequest, useRoleCreateRequest } from './roleCreateRequest'
import type { RoleKind } from './roleYaml'

export interface RoleCreateLauncherProps {
  kind: RoleKind
  /** Button text; defaults to "New Role"/"New ClusterRole". */
  label?: string
  /** Test hook; defaults to `rbac-role-create-open` (the Roles/ClusterRoles toolbars). */
  testId?: string
  /** Namespace for a Role and its access check; defaults to the active namespace. */
  namespace?: string | null
  variant?: 'default' | 'outline' | 'link'
  /** Called after a successful create; the sheet closes instead of offering "Bind this role…". */
  onCreated?: (target: BindTarget) => void
  /** Open on a command-palette request for this kind (only one launcher per kind should). */
  acceptRequests?: boolean
}

interface BindStep {
  target: BindTarget
  bindingKind: RoleBindingKind
}

export function RoleCreateLauncher({
  kind,
  label,
  testId = 'rbac-role-create-open',
  namespace,
  variant = 'default',
  onCreated,
  acceptRequests = false,
}: RoleCreateLauncherProps) {
  const [open, setOpen] = useState(false)
  const [bind, setBind] = useState<BindStep | undefined>(undefined)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const ns = namespace === undefined ? activeNamespace : namespace
  const title = kind === 'Role' ? 'New Role' : 'New ClusterRole'
  const gate = {
    apiGroup: 'rbac.authorization.k8s.io',
    verb: 'create',
    resource: kind === 'Role' ? 'roles' : 'clusterroles',
    namespace: kind === 'Role' ? ns : null,
  }

  // Command palette: take a fresh request for this kind once the access check has answered; a
  // denied identity keeps the disabled button (with its reason) and the request is dropped.
  const capability = useCapability(acceptRequests ? gate : null)
  const request = useRoleCreateRequest((s) => s.request)
  useEffect(() => {
    if (!acceptRequests || request?.kind !== kind) return
    if (capability.status === 'pending') return
    const fresh = takeRoleCreateRequest(kind)
    // eslint-disable-next-line react-hooks/set-state-in-effect -- opening on an external request
    if (fresh && capability.status === 'allowed') setOpen(true)
  }, [acceptRequests, request, kind, capability.status])

  const onOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) setBind(undefined)
  }

  return (
    <>
      <Gated {...gate}>
        <Button
          data-testid={testId}
          size="sm"
          variant={variant}
          className={variant === 'link' ? 'h-auto px-0' : undefined}
          onClick={() => setOpen(true)}
        >
          {label ?? title}
        </Button>
      </Gated>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-2xl">
          <SheetHeader>
            <SheetTitle>
              {bind ? `Bind ${bind.target.roleKind} ${bind.target.name}` : title}
            </SheetTitle>
          </SheetHeader>
          {open && bind && (
            <RoleBindingForm
              mode="create"
              kind={bind.bindingKind}
              initial={{
                name: `${bind.target.name}-binding`,
                namespace: bind.target.namespace ?? ns ?? undefined,
                roleRefKind: bind.target.roleKind,
                roleRefName: bind.target.name,
              }}
              onClose={() => onOpenChange(false)}
            />
          )}
          {open && !bind && (
            <RoleForm
              mode="create"
              kind={kind}
              {...(ns ? { namespace: ns } : {})}
              onClose={() => onOpenChange(false)}
              {...(onCreated
                ? {
                    onCreated: (target: BindTarget) => {
                      onOpenChange(false)
                      onCreated(target)
                    },
                  }
                : { onBind: (target, bindingKind) => setBind({ target, bindingKind }) })}
            />
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
