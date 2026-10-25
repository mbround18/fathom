/**
 * "Edit" entry point for an open Role/ClusterRole inspector (042-role-create), seeded from the
 * detail payload the inspector already holds (`RoleDetails`/`ClusterRoleDetails`); no extra read.
 */
import { useState } from 'react'

import { Button, Sheet, SheetContent, SheetHeader, SheetTitle } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import type { ClusterRoleDetails, RoleDetails } from '@/protocol/types'

import { RoleForm } from './RoleForm'
import type { RoleKind } from './roleYaml'

export interface RoleEditLauncherProps {
  kind: RoleKind
  existing: RoleDetails | ClusterRoleDetails
}

export function RoleEditLauncher({ kind, existing }: RoleEditLauncherProps) {
  const [open, setOpen] = useState(false)

  return (
    <>
      <Gated
        apiGroup="rbac.authorization.k8s.io"
        verb="update"
        resource={kind === 'Role' ? 'roles' : 'clusterroles'}
        namespace={'namespace' in existing ? existing.namespace : null}
      >
        <Button
          data-testid="rbac-role-edit-open"
          size="sm"
          variant="outline"
          onClick={() => setOpen(true)}
        >
          Edit
        </Button>
      </Gated>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-2xl">
          <SheetHeader>
            <SheetTitle>{`Edit ${kind} ${existing.name}`}</SheetTitle>
          </SheetHeader>
          {open && (
            <RoleForm mode="edit" kind={kind} existing={existing} onClose={() => setOpen(false)} />
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
