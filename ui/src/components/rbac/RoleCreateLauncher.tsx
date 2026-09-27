/**
 * "New Role"/"New ClusterRole" entry point (042-role-create), mounted in the Roles and
 * ClusterRoles list toolbars. Opens `RoleForm` (create) in a `Sheet`; after a successful create
 * the form's "Bind this role…" swaps the sheet to `RoleBindingForm` prefilled with the new role.
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Gated } from '@/lib/capabilities'
import { Sheet, SheetContent, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import { useStore } from '@/store'

import { RoleBindingForm, type RoleBindingKind } from './RoleBindingForm'
import { RoleForm, type BindTarget } from './RoleForm'
import type { RoleKind } from './roleYaml'

export interface RoleCreateLauncherProps {
  kind: RoleKind
}

interface BindStep {
  target: BindTarget
  bindingKind: RoleBindingKind
}

export function RoleCreateLauncher({ kind }: RoleCreateLauncherProps) {
  const [open, setOpen] = useState(false)
  const [bind, setBind] = useState<BindStep | undefined>(undefined)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const label = kind === 'Role' ? 'New Role' : 'New ClusterRole'

  const onOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) setBind(undefined)
  }

  return (
    <>
      <Gated
        apiGroup="rbac.authorization.k8s.io"
        verb="create"
        resource={kind === 'Role' ? 'roles' : 'clusterroles'}
        namespace={kind === 'Role' ? activeNamespace : null}
      >
        <Button data-testid="rbac-role-create-open" size="sm" onClick={() => setOpen(true)}>
          {label}
        </Button>
      </Gated>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-2xl">
          <SheetHeader>
            <SheetTitle>
              {bind ? `Bind ${bind.target.roleKind} ${bind.target.name}` : label}
            </SheetTitle>
          </SheetHeader>
          {open && bind && (
            <RoleBindingForm
              mode="create"
              kind={bind.bindingKind}
              initial={{
                name: `${bind.target.name}-binding`,
                namespace: bind.target.namespace ?? activeNamespace ?? undefined,
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
              onClose={() => onOpenChange(false)}
              onBind={(target, bindingKind) => setBind({ target, bindingKind })}
            />
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
