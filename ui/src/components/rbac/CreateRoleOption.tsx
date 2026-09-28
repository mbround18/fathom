/**
 * 043 US8 (FR-040/FR-041): "Create a new role…" at the end of a binding flow's role picker (the
 * RoleBinding form and the Assign role dialog). The flow swaps in 042's `RoleForm` with
 * `onCreated`, then comes back with the new role as its `roleRef`; nothing is created until the
 * operator applies each form. Gated like 042's "New Role" launcher.
 */
import { Button } from '@/components/ui/button'
import { Gated } from '@/lib/capabilities'

import type { RoleKind } from './roleYaml'

export const CREATE_ROLE_OPTION = 'Create a new role…'

export function CreateRoleOption({
  kind,
  namespace,
  onSelect,
  testId,
}: {
  kind: RoleKind
  namespace: string | null
  onSelect: () => void
  testId: string
}) {
  return (
    <Gated
      apiGroup="rbac.authorization.k8s.io"
      verb="create"
      resource={kind === 'Role' ? 'roles' : 'clusterroles'}
      namespace={kind === 'Role' ? namespace : null}
    >
      <Button
        type="button"
        data-testid={testId}
        size="sm"
        variant="link"
        className="h-auto self-start px-0"
        onClick={onSelect}
      >
        {CREATE_ROLE_OPTION}
      </Button>
    </Gated>
  )
}
