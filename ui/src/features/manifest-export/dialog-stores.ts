/**
 * 040-manifest-export: the open/close state of the export and kubeconfig dialogs, kept apart
 * from the components so either can be opened from menus, the selection bar or the header.
 */
import { create } from 'zustand'

import type { KubeconfigCredentialKind, ManifestObjectRef } from '@/protocol/types'
import type { AnyDescriptor } from '@/resources'
import type { ListRow } from '@/store'

import { compareRefs } from './clean'
import { manifestRefFor } from './kinds'

export interface ExportRequest {
  /** `FetchManifests` request id, fixed when the dialog opens. */
  id: string
  kind: string
  refs: ManifestObjectRef[]
  /** Where focus goes back on close. */
  returnFocus?: HTMLElement | null
}

interface ExportDialogStore {
  request: ExportRequest | null
  close: () => void
}

let nextRequest = 0

export const useExportDialog = create<ExportDialogStore>((set) => ({
  request: null,
  close: () => set({ request: null }),
}))

/** Open the export dialog for these rows of one list (sorted by namespace, then name). */
export function openManifestExport(
  descriptor: AnyDescriptor,
  rows: readonly ListRow[],
  returnFocus?: HTMLElement | null,
): void {
  const refs = rows
    .map((r) => manifestRefFor(descriptor, r))
    .filter((r): r is ManifestObjectRef => r !== undefined)
    .sort(compareRefs)
  if (refs.length === 0) return
  nextRequest += 1
  useExportDialog.setState({
    request: {
      id: `export-${String(Date.now())}-${String(nextRequest)}`,
      kind: refs[0]?.kind ?? descriptor.kind,
      refs,
      returnFocus: returnFocus ?? null,
    },
  })
}

interface KubeconfigExportStore {
  open: boolean
  context: string | null
  show: (context: string | null) => void
  close: () => void
}

export const useKubeconfigExport = create<KubeconfigExportStore>((set) => ({
  open: false,
  context: null,
  show: (context) => set({ open: true, context }),
  close: () => set({ open: false }),
}))

export function credentialLabel(c: KubeconfigCredentialKind): string {
  switch (c.type) {
    case 'Exec':
      return `Exec plugin (${c.data.command}): kept as-is, runs on the target machine`
    case 'AuthProvider':
      return `Auth provider (${c.data.name})`
    case 'Token':
      return 'Bearer token'
    case 'TokenFile':
      return 'Token file'
    case 'ClientCertificate':
      return 'Client certificate'
    case 'Basic':
      return 'Basic auth'
    case 'None':
      return 'No credentials'
  }
}
