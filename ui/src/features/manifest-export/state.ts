/**
 * 040-manifest-export: replies to `FetchManifests`, `PreviewKubeconfigExport` and
 * `ExportKubeconfig` (`store/handlers/system.ts`). Read-only on the cluster; the kubeconfig
 * slice only ever holds the redacted preview and the result of the write (FR-026).
 */
import type {
  FetchedManifest,
  KubeconfigExportPreview,
  ManifestFetchError,
  SystemEvent,
} from '@/protocol/types'

export interface ManifestFetchReply {
  manifests: FetchedManifest[]
  errors: ManifestFetchError[]
}

export interface KubeconfigPreviewState {
  /** Increments per reply so the dialog can tell a fresh reply from a stale one. */
  seq: number
  data?: KubeconfigExportPreview
  error?: string
}

export interface KubeconfigResultState {
  seq: number
  context: string
  ok: boolean
  path?: string
  embedded: boolean
  message?: string
}

export interface ManifestExportState {
  /** Keyed by the request id the dialog generated; the dialog drops its entry on close. */
  fetches: Record<string, ManifestFetchReply>
  kubeconfigPreview: KubeconfigPreviewState | null
  kubeconfigResult: KubeconfigResultState | null
  seq: number
}

/** Only this many replies are kept; older ones belong to dialogs that have closed. */
const MAX_FETCHES = 8

export function initialManifestExportState(): ManifestExportState {
  return { fetches: {}, kubeconfigPreview: null, kubeconfigResult: null, seq: 0 }
}

export function reduceManifestExport(s: ManifestExportState, event: SystemEvent): boolean {
  switch (event.type) {
    case 'ManifestsFetched': {
      s.fetches[event.data.request_id] = {
        manifests: event.data.manifests,
        errors: event.data.errors,
      }
      const keys = Object.keys(s.fetches)
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by request id
      for (const k of keys.slice(0, Math.max(0, keys.length - MAX_FETCHES))) delete s.fetches[k]
      return true
    }
    case 'KubeconfigExportPreviewed':
      s.seq += 1
      s.kubeconfigPreview = { seq: s.seq, data: event.data }
      return true
    case 'KubeconfigExportFinished': {
      s.seq += 1
      const d = event.data
      // Also answers a preview that failed (unknown context, hosted refusal): the dialog
      // compares `seq` with the one it saw when it asked.
      s.kubeconfigResult = {
        seq: s.seq,
        context: d.context,
        ok: d.ok,
        embedded: d.embedded_credentials_written,
        ...(d.path !== null ? { path: d.path } : {}),
        ...(d.message !== null ? { message: d.message } : {}),
      }
      return true
    }
    default:
      return false
  }
}
