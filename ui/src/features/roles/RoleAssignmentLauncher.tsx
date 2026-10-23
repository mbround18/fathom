/**
 * 033 US4: "Assign from directory" button beside the binding-create buttons, wrapped in
 * `<Gated>` so it is disabled (with the reason) when RBAC forbids creating bindings. Also asks
 * the backend for the orphaned-subject join once per mount.
 */
import { useEffect, useState } from 'react'

import { Button, Sheet, SheetContent, SheetHeader, SheetTitle } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import { useStore } from '@/store'

import { RoleAssignmentDialog } from './RoleAssignmentDialog'

export function RoleAssignmentLauncher({
  kind,
  issuerId: given,
}: {
  kind: 'RoleBinding' | 'ClusterRoleBinding'
  /**
   * 045 T043/FR-030: the issuer this launcher belongs to, given rather than guessed. Reading
   * `issuers[0]` was wrong the moment a second issuer existed: the directory it offers, the
   * orphaned-subject join it asks for and the prefixes the preview renders all belong to one
   * issuer, so picking the first one silently bound subjects from another provider's directory.
   */
  issuerId?: string
}) {
  const [open, setOpen] = useState(false)
  const namespace = useStore((s) => s.session.activeNamespace)
  const context = useStore((s) => s.session.activeContext)
  const contexts = useStore((s) => s.identity.config.contexts)
  const issuers = useStore((s) => s.identity.config.issuers)
  const send = useStore((s) => s.send)
  const ns = kind === 'RoleBinding' ? namespace : null
  // Without one given, the issuer is the one this context authenticates with — the binding screens
  // are about a cluster, and that is the provider whose subjects it accepts. A single configured
  // issuer is the same answer by another route; with several and no context choice there is no
  // issuer to guess, and `RoleAssignmentDialog`'s own picker is what resolves it.
  const bound = contexts.find((c) => c.context_name === context)?.issuer_id ?? undefined
  const issuerId = given ?? bound ?? (issuers.length === 1 ? issuers[0]?.id : undefined)

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
          {open && (
            <RoleAssignmentDialog
              namespace={ns}
              issuerId={issuerId}
              onClose={() => setOpen(false)}
            />
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
