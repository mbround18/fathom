/**
 * 040 US1/US2 (FR-002, FR-003, FR-014..FR-017): the "Export YAML" dialog for one row or a
 * checked selection. It reads the objects fresh (`FetchManifests`, read-only), then shows the
 * YAML with a clean toggle, Copy and Download, the object count, and per-object errors and
 * warnings. Replies are matched by request id, so a late reply for a closed dialog is ignored.
 *
 * Test ids: `manifest-export-dialog`, `manifest-export-count`, `manifest-export-yaml`,
 * `manifest-export-error`, `manifest-warning`, plus the `ManifestActions` ids.
 */
import { useEffect, useMemo, useState } from 'react'

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@companion/ui'
import type { ManifestObjectRef } from '@/protocol/types'
import { useStore } from '@/store'

import { MANIFEST_EXPORT_MAX_OBJECTS } from './limits'
import { exportFilename, renderBundle, renderManifest } from './clean'
import { useExportDialog, type ExportRequest } from './dialog-stores'
import { ManifestActions } from './ManifestActions'

export function ManifestExportDialogHost() {
  const request = useExportDialog((s) => s.request)
  const close = useExportDialog((s) => s.close)
  if (!request) return null
  return <ManifestExportDialog key={request.id} request={request} onClose={close} />
}

function ManifestExportDialog({
  request,
  onClose,
}: {
  request: ExportRequest
  onClose: () => void
}) {
  const send = useStore((s) => s.send)
  const [clean, setClean] = useState(true)
  const tooMany = request.refs.length > MANIFEST_EXPORT_MAX_OBJECTS
  const id = request.id
  useEffect(() => {
    if (tooMany) return
    send({ type: 'FetchManifests', data: { request_id: id, objects: request.refs } })
  }, [id, request.refs, send, tooMany])
  const reply = useStore((s) => s.manifestExport.fetches[id])

  const rendered = useMemo(() => {
    if (!reply) return undefined
    const docs: string[] = []
    const warnings: { ref: ManifestObjectRef; text: string }[] = []
    const errors = reply.errors.map((e) => ({ ref: request.refs[e.index], message: e.message }))
    for (const m of reply.manifests) {
      const ref = request.refs[m.index]
      if (!ref) continue
      try {
        const r = renderManifest(m.raw_json, { clean })
        docs.push(r.yaml)
        for (const w of r.warnings) warnings.push({ ref, text: w })
      } catch (e) {
        errors.push({ ref, message: `Could not render: ${String(e)}` })
      }
    }
    return { yaml: renderBundle(docs), count: docs.length, warnings, errors }
  }, [clean, reply, request.refs])

  const total = request.refs.length
  const [only] = request.refs
  const title =
    total === 1 && only
      ? `Export ${only.kind} ${only.name}`
      : `Export ${String(total)} ${request.kind} objects`
  const filename = exportFilename(
    request.kind,
    rendered?.count === 1 && only ? [only] : request.refs,
  )

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent
        data-testid="manifest-export-dialog"
        onCloseAutoFocus={(e) => {
          if (request.returnFocus?.isConnected) {
            e.preventDefault()
            request.returnFocus.focus()
          }
        }}
        className="flex max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] flex-col gap-3 sm:max-w-3xl"
      >
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider uppercase">{title}</DialogTitle>
          <DialogDescription>
            Read fresh from the cluster. Clean output drops status and server-written fields so it
            can be re-applied.
          </DialogDescription>
        </DialogHeader>
        {tooMany ? (
          <p data-testid="manifest-export-error" className="font-mono text-destructive">
            {String(total)} objects selected: the export limit is{' '}
            {String(MANIFEST_EXPORT_MAX_OBJECTS)}. Nothing was read.
          </p>
        ) : (
          <>
            <p data-testid="manifest-export-count" className="font-mono text-muted-foreground">
              {rendered
                ? `${String(rendered.count)} of ${String(total)} objects`
                : `Reading ${String(total)} objects…`}
            </p>
            {rendered?.errors.map((e, i) => (
              <p
                key={`e${String(i)}`}
                data-testid="manifest-export-error"
                className="font-mono break-all text-destructive"
              >
                {e.ref ? `${e.ref.namespace ? `${e.ref.namespace}/` : ''}${e.ref.name}: ` : ''}
                {e.message}
              </p>
            ))}
            {rendered?.warnings.map((w, i) => (
              <p
                key={`w${String(i)}`}
                data-testid="manifest-warning"
                className="font-mono break-all text-amber-700 dark:text-amber-400"
              >
                {total > 1 ? `${w.ref.name}: ` : ''}
                {w.text}
              </p>
            ))}
            <ManifestActions
              text={rendered?.yaml}
              filename={filename}
              clean={{ value: clean, onChange: setClean }}
            />
            <pre
              data-testid="manifest-export-yaml"
              // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- scrollable region must be keyboard reachable
              tabIndex={0}
              className="min-h-24 flex-1 overflow-auto border border-border bg-background p-2 font-mono text-xs leading-relaxed whitespace-pre"
            >
              {rendered?.yaml ?? ''}
            </pre>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
