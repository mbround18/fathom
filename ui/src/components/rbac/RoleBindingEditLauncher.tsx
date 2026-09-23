/**
 * "Edit" entry point for an existing RoleBinding/ClusterRoleBinding (011-rbac-oidc-administration
 * US3 T032). Standalone, mirroring `RoleBindingCreateLauncher.tsx`, so the list descriptors that
 * own their own files (`ui/src/resources/rolebindings.tsx`/`clusterrolebindings.tsx`) can drop it
 * into a detail view without this module reaching into them.
 *
 * Mounts `RoleBindingForm` (edit mode) in a `Sheet`, seeded from the already-loaded detail object
 * (`RoleBindingDetails`/`ClusterRoleBindingDetails`, from `SelectRoleBinding`/
 * `SelectClusterRoleBinding`'s reply) — no extra read is issued here.
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Sheet, SheetContent, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import type { ClusterRoleBindingDetails, RoleBindingDetails } from '@/protocol/types'

import { RoleBindingForm, type RoleBindingKind } from './RoleBindingForm'

export interface RoleBindingEditLauncherProps {
  kind: RoleBindingKind
  existing: RoleBindingDetails | ClusterRoleBindingDetails
}

export function RoleBindingEditLauncher({ kind, existing }: RoleBindingEditLauncherProps) {
  const [open, setOpen] = useState(false)
  const label = kind === 'RoleBinding' ? 'Edit RoleBinding' : 'Edit ClusterRoleBinding'

  return (
    <>
      <Button data-testid="rbac-binding-edit-open" size="sm" variant="outline" onClick={() => setOpen(true)}>
        Edit
      </Button>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-xl">
          <SheetHeader>
            <SheetTitle>{label}</SheetTitle>
          </SheetHeader>
          {open && (
            <RoleBindingForm mode="edit" kind={kind} existing={existing} onClose={() => setOpen(false)} />
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
