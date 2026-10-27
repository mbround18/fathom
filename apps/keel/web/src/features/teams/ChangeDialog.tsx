/**
 * The preview → confirm dialog for every governance change (spec 055 US6 T038).
 *
 * It shows the server's preview as is: the summary, every affected user and object, and whether
 * the change needs a passkey. Nothing changes until Confirm; Cancel and Escape send nothing. A
 * refusal is shown verbatim. A new API token's secret is shown here once and is gone when the
 * dialog closes.
 */
import { CopyIcon, FingerprintIcon } from '@phosphor-icons/react'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@companion/ui'

import { RefusalText } from './parts'
import { useGovernance, type Change } from './store'

function copy(text: string): void {
  try {
    void globalThis.navigator.clipboard.writeText(text).catch(() => undefined)
  } catch {
    /* No clipboard (insecure context): the secret is selectable text anyway. */
  }
}

function describe(change: Change): string {
  switch (change.phase) {
    case 'asking':
      return 'Asking Keel what this changes…'
    case 'preview':
    case 'stepping':
      return change.preview?.summary ?? ''
    case 'confirming':
      return 'Applying…'
    case 'refused':
      return 'Keel refused this change. Nothing was changed.'
    case 'done':
      return change.created ? 'Token created. Copy the secret now: Keel will not show it again.' : 'Done.'
  }
}

function Body({ change }: { change: Change }) {
  const { preview } = change
  return (
    <>
      {preview && (change.phase === 'preview' || change.phase === 'confirming') && (
        <section aria-label="Affected" className="flex min-w-0 flex-col gap-1">
          <h3 className="text-xs text-muted-foreground">
            Affects {preview.affected.length === 0 ? 'nothing else' : String(preview.affected.length)}
          </h3>
          {preview.affected.length > 0 && (
            <ul data-affected className="max-h-48 min-w-0 overflow-y-auto border border-border">
              {preview.affected.map((a, i) => (
                <li key={i} className="border-t border-border px-2 py-0.5 text-xs break-words first:border-t-0">
                  {a}
                </li>
              ))}
            </ul>
          )}
          {preview.step_up && (
            <p data-step-up className="flex items-center gap-1 text-xs text-status-warn">
              <FingerprintIcon aria-hidden className="size-3.5" />
              Needs your passkey.
            </p>
          )}
        </section>
      )}
      {change.refusal && <RefusalText refusal={change.refusal} />}
      {change.created && (
        <div className="flex min-w-0 flex-col gap-1">
          <p className="text-xs">
            Secret for <span className="font-medium">{change.created.token.name}</span>:
          </p>
          <div className="flex min-w-0 items-start gap-1">
            <code data-token-secret className="min-w-0 flex-1 border border-border p-1 font-mono text-xs break-all select-all">
              {change.created.secret}
            </code>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label="Copy secret"
              onClick={() => change.created && copy(change.created.secret)}
            >
              <CopyIcon />
            </Button>
          </div>
        </div>
      )}
    </>
  )
}

export function ChangeDialog() {
  const change = useGovernance((s) => s.change)
  const confirm = useGovernance((s) => s.confirm)
  const dismiss = useGovernance((s) => s.dismiss)
  // While the passkey prompt is up, this dialog steps aside so only one dialog holds focus.
  const open = change !== null && change.phase !== 'stepping'
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) dismiss()
      }}
    >
      <DialogContent data-change-dialog={change?.phase} className="sm:max-w-md">
        {change && (
          <>
            <DialogHeader>
              <DialogTitle>{change.title}</DialogTitle>
              <DialogDescription data-summary>{describe(change)}</DialogDescription>
            </DialogHeader>
            <Body change={change} />
            <DialogFooter>
              {change.phase === 'done' || change.phase === 'refused' ? (
                <Button onClick={dismiss}>Close</Button>
              ) : (
                <>
                  <Button type="button" variant="ghost" onClick={dismiss}>
                    Cancel
                  </Button>
                  <Button disabled={change.phase !== 'preview'} onClick={() => void confirm()}>
                    Confirm
                  </Button>
                </>
              )}
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
