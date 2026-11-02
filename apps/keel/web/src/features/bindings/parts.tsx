/**
 * Pieces the bindings and receivers pages share (spec 055 T052): the workspace picker, the
 * confirm dialog for a `Preview`, and the error line, on the app's shared primitives.
 */
import type { ReactNode } from 'react'
import { ConfirmDialog, ErrorLine as Alert, Field as FormField } from '@companion/ui/kit'

import { GrantWorkspacePicker } from '@/app/WorkspacePicker'

import { useRepos } from './store'

/** A labelled form field. */
export function Field({ label, children, id }: { label: string; children: ReactNode; id: string }) {
  return (
    <FormField label={label} htmlFor={id}>
      {children}
    </FormField>
  )
}

/** Picks one of the signed-in user's workspaces and loads its lists. */
export function WorkspacePicker() {
  const workspace = useRepos((s) => s.workspace)
  const load = useRepos((s) => s.load)
  return <GrantWorkspacePicker workspace={workspace} load={load} />
}

export function ErrorLine() {
  const error = useRepos((s) => s.error)
  const clear = useRepos((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}

/** The confirm dialog for the pending preview. */
export function PreviewDialog() {
  const preview = useRepos((s) => s.preview)
  const confirm = useRepos((s) => s.confirm)
  const cancel = useRepos((s) => s.cancel)
  return <ConfirmDialog preview={preview} onCancel={cancel} onConfirm={() => void confirm()} />
}
