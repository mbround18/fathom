/**
 * YAML edit tab (US1, T023/T025): editable buffer seeded from the same `raw_json` the read-only
 * YAML view rendered, with a "Review changes" step (diff against the live object) and an
 * explicit confirm-to-apply (constitution I: no optimistic UI, every mutation confirmed,
 * backend errors shown verbatim).
 *
 * Routed through `ConfirmDialog` directly rather than `ActionDialogHost`/`runAction`: those are
 * built around per-row bulk actions whose only success signal is `CommandFinished`, while an
 * apply's success is the `ResourceApplied` stream event (it also carries the new
 * `resourceVersion`, which `CommandFinished` does not). `ConfirmDialog` gives the same explicit-
 * confirmation UI and test hooks without forcing this single-object full-replace flow through
 * that row/descriptor abstraction.
 */
import { useEffect, useRef, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { ObjectRef } from '@/resources'
import { detailKey, useStore } from '@/store'
import { cn } from '@/lib/utils'

import { YamlDiffView } from './YamlDiffView'

const STATUS_LABEL: Record<string, string> = {
  conflict: 'Someone else changed this object first',
  deleted: 'This object no longer exists',
  error: 'Apply failed',
}

export function YamlEditTab({ target, raw }: { target: ObjectRef; raw: string | undefined }) {
  const context = useStore((s) => s.session.activeContext)
  const key = detailKey(context, target.kind, target.namespace, target.name)
  const session = useStore((s) => s.editSessions[key])
  const openEditSession = useStore((s) => s.openEditSession)
  const updateEditedYaml = useStore((s) => s.updateEditedYaml)
  const reviewEditDiff = useStore((s) => s.reviewEditDiff)
  const discardEditSession = useStore((s) => s.discardEditSession)
  const applyEditSession = useStore((s) => s.applyEditSession)
  const inspect = useStore((s) => s.inspect)

  const [confirmOpen, setConfirmOpen] = useState(false)
  const pendingReload = useRef(false)

  // Seed (or refresh, after an explicit Reload) the session once `raw` is available.
  useEffect(() => {
    if (raw === undefined) return
    if (!session || pendingReload.current) {
      openEditSession(target, raw)
      pendingReload.current = false
    }
    // Only re-run when the identity or the raw payload changes; `session` itself is derived.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [raw, target.kind, target.namespace, target.name, openEditSession])

  if (raw === undefined || !session) {
    return <p className="p-3 text-xs text-muted-foreground">Loading YAML…</p>
  }

  const reload = () => {
    pendingReload.current = true
    inspect(target.kind, target.namespace ?? '', target.name)
  }

  const canReview = session.dirty && session.status !== 'applying'
  const showDiff = session.status === 'diffing'
  const busy = session.status === 'applying'
  const blocked = session.status === 'conflict' || session.status === 'deleted'

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {session.status !== 'editing' && session.status !== 'diffing' && (
        <div
          data-testid="yaml-apply-status"
          data-status={session.status}
          className={cn(
            'flex flex-col gap-1 border-b border-border px-3 py-2 text-xs',
            session.status === 'applied' && 'text-status-ok',
            (blocked || session.status === 'error') && 'text-destructive',
          )}
        >
          {session.status === 'applying' && <p>Applying…</p>}
          {session.status === 'applied' && <p>Applied.</p>}
          {(blocked || session.status === 'error') && (
            <>
              <p className="font-semibold">{STATUS_LABEL[session.status]}</p>
              {session.lastError && (
                <p data-testid="yaml-apply-error" className="font-mono break-words">
                  {session.lastError}
                </p>
              )}
              {blocked && (
                <Button
                  data-testid="yaml-reload"
                  size="sm"
                  variant="outline"
                  className="w-fit"
                  onClick={reload}
                >
                  Reload latest
                </Button>
              )}
            </>
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col">
        {showDiff ? (
          <YamlDiffView session={session} />
        ) : (
          <Textarea
            data-testid="yaml-edit-textarea"
            value={session.editedYaml}
            onChange={(e) => updateEditedYaml(key, e.target.value)}
            disabled={busy || blocked}
            spellCheck={false}
            className="min-h-0 flex-1 resize-none rounded-none border-0 p-3 font-mono text-xs leading-relaxed focus-visible:ring-0"
          />
        )}
      </div>

      <div className="flex shrink-0 items-center justify-end gap-2 border-t border-border px-3 py-2">
        {session.dirty && (
          <Button
            data-testid="yaml-discard"
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => discardEditSession(key)}
          >
            Discard
          </Button>
        )}
        {showDiff ? (
          <Button
            data-testid="yaml-apply"
            size="sm"
            disabled={busy || blocked || !session.dirty}
            onClick={() => setConfirmOpen(true)}
          >
            Apply
          </Button>
        ) : (
          <Button
            data-testid="yaml-review"
            size="sm"
            disabled={!canReview}
            onClick={() => reviewEditDiff(key)}
          >
            Review changes
          </Button>
        )}
      </div>

      <ConfirmDialog
        open={confirmOpen}
        title={`Apply changes to ${target.name}`}
        items={[{ namespace: target.namespace, name: target.name }]}
        detail="The edited YAML replaces the live object. This cannot be undone."
        confirmLabel="Apply"
        onConfirm={() => {
          setConfirmOpen(false)
          void applyEditSession(key)
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  )
}
