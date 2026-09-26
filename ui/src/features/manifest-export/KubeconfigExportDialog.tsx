/**
 * 040 US4 (FR-021..FR-032): export one kubeconfig context as a standalone file.
 *
 * The UI never holds a credential: it shows the backend's redacted preview, the credential type
 * and which secret fields exist, and asks the backend to write the file (0600) after a separate
 * confirmation that names the context, the path and whether credentials are included. There is
 * deliberately no Copy here: the unredacted file only ever exists on disk (FR-026).
 *
 * Test ids: `kubeconfig-export-dialog`, `kubeconfig-context`, `kubeconfig-credential`,
 * `kubeconfig-embedded`, `kubeconfig-include-credentials`, `kubeconfig-file-refs`,
 * `kubeconfig-warning`, `kubeconfig-preview`, `kubeconfig-path`, `kubeconfig-overwrite`,
 * `kubeconfig-export`, `kubeconfig-result`, `kubeconfig-error`; the confirm step is
 * `confirm-dialog` / `confirm-submit`.
 */
import { WarningIcon } from '@phosphor-icons/react'
import { useEffect, useId, useRef, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

import { credentialLabel, useKubeconfigExport } from './dialog-stores'

export function KubeconfigExportDialogHost() {
  const open = useKubeconfigExport((s) => s.open)
  const context = useKubeconfigExport((s) => s.context)
  const close = useKubeconfigExport((s) => s.close)
  const active = useStore((s) => s.session.activeContext)
  if (!open) return null
  return <KubeconfigExportDialog initial={context ?? active ?? ''} onClose={close} />
}

function KubeconfigExportDialog({ initial, onClose }: { initial: string; onClose: () => void }) {
  const send = useStore((s) => s.send)
  const contexts = useStore((s) => s.session.contexts)
  const slice = useStore((s) => s.manifestExport)
  const [context, setContext] = useState(initial)
  // Replies count only when newer than the ask (seq), so a stale reply is never shown.
  const [previewSince, setPreviewSince] = useState(slice.seq)
  const [writeSince, setWriteSince] = useState<number | null>(null)
  const [path, setPath] = useState('')
  const [include, setInclude] = useState(false)
  const [overwrite, setOverwrite] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const pathEdited = useRef(false)
  const ids = { ctx: useId(), path: useId(), include: useId(), overwrite: useId() }

  const askPreview = (ctx: string) => {
    setPreviewSince(useStore.getState().manifestExport.seq)
    setWriteSince(null)
    setInclude(false)
    if (ctx !== '') send({ type: 'PreviewKubeconfigExport', data: { context: ctx } })
  }
  // Ask once on open; later asks come from the context select.
  const asked = useRef(false)
  useEffect(() => {
    if (asked.current) return
    asked.current = true
    askPreview(initial)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const newer = <T extends { seq: number }>(v: T | null, since: number | null): T | undefined =>
    v !== null && since !== null && v.seq > since ? v : undefined
  const preview = newer(slice.kubeconfigPreview, previewSince)?.data
  const previewForContext = preview?.context === context ? preview : undefined
  const previewFailure =
    writeSince === null ? newer(slice.kubeconfigResult, previewSince) : undefined
  const previewError =
    previewForContext === undefined && previewFailure && !previewFailure.ok
      ? previewFailure.message
      : undefined
  const writeResult = newer(slice.kubeconfigResult, writeSince)
  const writing = writeSince !== null && writeResult === undefined

  useEffect(() => {
    if (previewForContext && !pathEdited.current) setPath(previewForContext.suggested_path)
  }, [previewForContext])

  const embedded = previewForContext?.embedded_secret_fields ?? []
  const includeCreds = include && embedded.length > 0
  const canExport = previewForContext !== undefined && path.trim() !== '' && !writing

  const write = () => {
    setConfirming(false)
    setWriteSince(useStore.getState().manifestExport.seq)
    send({
      type: 'ExportKubeconfig',
      data: { context, path: path.trim(), include_embedded_credentials: includeCreds, overwrite },
    })
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent
        data-testid="kubeconfig-export-dialog"
        className="flex max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] flex-col gap-3 overflow-y-auto sm:max-w-2xl"
      >
        <DialogHeader>
          <DialogTitle className="font-display tracking-wider uppercase">
            Export kubeconfig
          </DialogTitle>
          <DialogDescription>
            Writes one context, its cluster and its user to a standalone file (owner-only
            permissions). Nothing is sent anywhere else.
          </DialogDescription>
        </DialogHeader>

        <label htmlFor={ids.ctx} className="flex flex-col gap-1 font-mono">
          <span className="text-muted-foreground">Context</span>
          <select
            id={ids.ctx}
            data-testid="kubeconfig-context"
            value={context}
            onChange={(e) => {
              setContext(e.target.value)
              pathEdited.current = false
              askPreview(e.target.value)
            }}
            className="h-[max(2rem,var(--touch-target))] border border-input bg-background px-2 font-mono text-xs"
          >
            {!contexts.includes(context) && <option value={context}>{context || 'none'}</option>}
            {contexts.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>

        {previewError !== undefined && (
          <p data-testid="kubeconfig-error" className="font-mono break-all text-destructive">
            {previewError}
          </p>
        )}
        {previewForContext === undefined && previewError === undefined && (
          <p className="font-mono text-muted-foreground">Reading kubeconfig…</p>
        )}

        {previewForContext && (
          <>
            <p data-testid="kubeconfig-credential" className="font-mono">
              <span className="text-muted-foreground">Credential: </span>
              {credentialLabel(previewForContext.credential)}
            </p>
            {embedded.length > 0 && (
              <div
                data-testid="kubeconfig-embedded"
                className="flex flex-col gap-2 border border-amber-600/40 bg-amber-500/10 p-2 text-amber-700 dark:text-amber-400"
              >
                <p className="flex items-start gap-2">
                  <WarningIcon aria-hidden="true" className="mt-0.5 shrink-0" />
                  <span>
                    This user has embedded credentials ({embedded.join(', ')}). They are left out
                    unless you include them; anyone with the file can then act as this user.
                  </span>
                </p>
                <label htmlFor={ids.include} className="flex cursor-pointer items-center gap-2">
                  <Checkbox
                    id={ids.include}
                    data-testid="kubeconfig-include-credentials"
                    checked={include}
                    onCheckedChange={(v) => setInclude(v === true)}
                  />
                  Include embedded credentials
                </label>
              </div>
            )}
            {previewForContext.file_references.length > 0 && (
              <div data-testid="kubeconfig-file-refs" className="font-mono">
                <p className="text-muted-foreground">Not portable (file paths kept as-is):</p>
                <ul className="list-inside list-disc break-all">
                  {previewForContext.file_references.map((f) => (
                    <li key={f}>{f}</li>
                  ))}
                </ul>
              </div>
            )}
            {previewForContext.warnings.map((w) => (
              <p
                key={w}
                data-testid="kubeconfig-warning"
                className="font-mono break-all text-amber-700 dark:text-amber-400"
              >
                {w}
              </p>
            ))}
            <pre
              data-testid="kubeconfig-preview"
              // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- scrollable region must be keyboard reachable
              tabIndex={0}
              className="max-h-64 min-h-16 overflow-auto border border-border bg-background p-2 font-mono text-xs leading-relaxed whitespace-pre"
            >
              {previewForContext.redacted_yaml}
            </pre>
            <label htmlFor={ids.path} className="flex flex-col gap-1 font-mono">
              <span className="text-muted-foreground">Write to</span>
              <Input
                id={ids.path}
                data-testid="kubeconfig-path"
                value={path}
                spellCheck={false}
                onChange={(e) => {
                  pathEdited.current = true
                  setPath(e.target.value)
                }}
                className="font-mono"
              />
            </label>
            <label
              htmlFor={ids.overwrite}
              className="flex cursor-pointer items-center gap-2 font-mono"
            >
              <Checkbox
                id={ids.overwrite}
                data-testid="kubeconfig-overwrite"
                checked={overwrite}
                onCheckedChange={(v) => setOverwrite(v === true)}
              />
              Overwrite if the file exists
            </label>
          </>
        )}

        {writeResult && (
          <p
            data-testid="kubeconfig-result"
            data-ok={writeResult.ok ? 'true' : 'false'}
            className={
              writeResult.ok
                ? 'font-mono break-all text-accent-neon'
                : 'font-mono break-all text-destructive'
            }
          >
            {writeResult.ok
              ? `Wrote ${writeResult.path ?? path} (0600)${writeResult.embedded ? ' with embedded credentials' : ', no embedded credentials'}.`
              : writeResult.message}
          </p>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={onClose} data-testid="kubeconfig-close">
            {writeResult?.ok ? 'Done' : 'Cancel'}
          </Button>
          {!writeResult?.ok && (
            <Button
              data-testid="kubeconfig-export"
              disabled={!canExport}
              onClick={() => setConfirming(true)}
            >
              Export…
            </Button>
          )}
        </DialogFooter>

        <ConfirmDialog
          open={confirming}
          title="Write kubeconfig file?"
          items={[{ namespace: null, name: context }]}
          detail={
            <span data-testid="kubeconfig-confirm-detail" className="flex flex-col gap-1">
              <span className="break-all">To: {path.trim()}</span>
              <span>
                {includeCreds
                  ? `Includes embedded credentials (${embedded.join(', ')}).`
                  : embedded.length > 0
                    ? 'Embedded credentials are left out.'
                    : 'No embedded credentials.'}
              </span>
              {overwrite && <span>Replaces the file if it exists.</span>}
            </span>
          }
          destructive={includeCreds}
          confirmLabel="Write file"
          onConfirm={write}
          onCancel={() => setConfirming(false)}
        />
      </DialogContent>
    </Dialog>
  )
}
