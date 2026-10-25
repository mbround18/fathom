/**
 * Delete a stack (US3, T062), admin only. It asks the server for a `delete` preview with the
 * directory kept; removing the directory is a switch in the preview, off by default, and named
 * there when on. The preview dialog also asks for the stack name typed, since delete is
 * destructive.
 */
import { TrashIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'

import { useSession } from '@/app/session-store'
import { useUntrusted } from '@/features/network/model'

import { useOps } from './ops-store'

export function DeleteStack({ stack }: { stack: string }) {
  const admin = useSession((s) => s.me?.role === 'admin')
  const requestPreview = useOps((s) => s.requestPreview)
  const blocked = useUntrusted()
  if (!admin) return null
  return (
    <Button
      variant="outline"
      size="sm"
      data-action="delete"
      disabled={blocked !== null}
      title={blocked ?? undefined}
      data-blocked={blocked === null ? undefined : 'untrusted_network'}
      className="text-status-err"
      onClick={() => requestPreview({ kind: 'op', stack, op: { kind: 'delete', options: { remove_directory: false } } })}
    >
      <TrashIcon />
      Delete
    </Button>
  )
}
