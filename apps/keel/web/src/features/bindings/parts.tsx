/**
 * Pieces the bindings and receivers pages share (spec 055 T052): the current workspace's loader, the
 * confirm dialog for a `Preview`, and the error line, on the app's shared primitives.
 */
import type { ReactNode } from 'react'
import { ConfirmDialog, ErrorLine as Alert, Field as FormField } from '@companion/ui/kit'

import { CurrentWorkspace } from '@/app/WorkspacePicker'

import { useRepos } from './store'

/** A labelled form field. */
export function Field({ label, children, id }: { label: string; children: ReactNode; id: string }) {
  return (
    <FormField label={label} htmlFor={id}>
      {children}
    </FormField>
  )
}

/** Loads the current workspace's lists (spec 066: the shell's switcher picks it). */
export function WorkspacePicker() {
  const load = useRepos((s) => s.load)
  return <CurrentWorkspace load={load} />
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
