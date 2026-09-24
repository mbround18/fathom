/**
 * 033 US4: "Assign from directory" button beside the binding-create buttons, wrapped in
 * `<Gated>` so it is disabled (with the reason) when RBAC forbids creating bindings. Also asks
 * the backend for the orphaned-subject join once per mount.
 */
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Sheet, SheetContent, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import { Gated } from '@/lib/capabilities'
import { useStore } from '@/store'

import { RoleAssignmentDialog } from './RoleAssignmentDialog'

export function RoleAssignmentLauncher({ kind }: { kind: 'RoleBinding' | 'ClusterRoleBinding' }) {
  const [open, setOpen] = useState(false)
  const namespace = useStore((s) => s.session.activeNamespace)
  const context = useStore((s) => s.session.activeContext)
  const issuerId = useStore((s) => s.identity.config.issuers[0]?.id)
  const send = useStore((s) => s.send)
  const ns = kind === 'RoleBinding' ? namespace : null

  useEffect(() => {
    if (issuerId && context) {
      send({ type: 'RefreshOrphanedSubjects', data: { context, issuer_id: issuerId } })
    }
  }, [issuerId, context, send])

  if (!issuerId) return null
  return (
    <>
      <Gated
        apiGroup="rbac.authorization.k8s.io"
        verb="create"
        resource={kind === 'RoleBinding' ? 'rolebindings' : 'clusterrolebindings'}
        namespace={ns}
      >
        <Button
          data-testid="role-assignment-open"
          size="sm"
          variant="outline"
          onClick={() => setOpen(true)}
        >
          Assign from directory
        </Button>
      </Gated>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-[calc(100vw-2rem)] sm:max-w-xl">
          <SheetHeader>
            <SheetTitle>Assign role</SheetTitle>
          </SheetHeader>
          {open && <RoleAssignmentDialog namespace={ns} onClose={() => setOpen(false)} />}
        </SheetContent>
      </Sheet>
    </>
  )
}
