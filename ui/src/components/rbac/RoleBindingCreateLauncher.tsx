/**
 * "New RoleBinding"/"New ClusterRoleBinding" entry point (011-rbac-oidc-administration US2
 * T029). Standalone so the list descriptors that own their own files
 * (`ui/src/resources/rolebindings.tsx`/`clusterrolebindings.tsx`) can drop it in without this
 * module reaching into them.
 *
 * Mounts `RoleBindingForm` (create mode) in a `Sheet` (the established modal-mounting
 * convention for a form flow, alongside `ConfirmDialog`/`ScaleDialog` for confirm-only prompts —
 * see `ActionDialogHost.tsx`).
 */
import { useState } from 'react'

import { Button, Sheet, SheetContent, SheetHeader, SheetTitle } from '@companion/ui'
import { Gated } from '@/lib/capabilities'

import { useStore } from '@/store'

import { RoleBindingForm, type RoleBindingKind } from './RoleBindingForm'

export interface RoleBindingCreateLauncherProps {
  kind: RoleBindingKind
}

export function RoleBindingCreateLauncher({ kind }: RoleBindingCreateLauncherProps) {
  const [open, setOpen] = useState(false)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const label = kind === 'RoleBinding' ? 'New RoleBinding' : 'New ClusterRoleBinding'

  return (
    <>
      <Gated
        apiGroup="rbac.authorization.k8s.io"
        verb="create"
        resource={kind === 'RoleBinding' ? 'rolebindings' : 'clusterrolebindings'}
        namespace={kind === 'RoleBinding' ? activeNamespace : null}
      >
        <Button data-testid="rbac-binding-create-open" size="sm" onClick={() => setOpen(true)}>
          {label}
        </Button>
      </Gated>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-xl">
          <SheetHeader>
            <SheetTitle>{label}</SheetTitle>
          </SheetHeader>
          {open && <RoleBindingForm mode="create" kind={kind} onClose={() => setOpen(false)} />}
        </SheetContent>
      </Sheet>
    </>
  )
}
