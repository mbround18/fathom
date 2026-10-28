/**
 * The preview → confirm dialog for every governance change (spec 055 US6 T038).
 *
 * It shows the server's preview as is: the summary, every affected user and object, and whether
 * the change needs a passkey. Nothing changes until Confirm; Cancel and Escape send nothing. A
 * refusal is shown verbatim. A new API token's secret is shown here once and is gone when the
 * dialog closes.
 */
import { Button } from '@companion/ui'

import { PreviewDetails } from '@/ui/ConfirmDialog'
import { CopyBlock } from '@/ui/CopyBlock'
import { Dialog } from '@/ui/Dialog'

import { RefusalText } from './parts'
import { useGovernance, type Change } from './store'

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
        <>
          {preview.affected.length === 0 && <p className="text-xs text-muted-foreground">Affects nothing else.</p>}
          <PreviewDetails preview={preview} stepUp="Needs your passkey." />
        </>
      )}
      {change.refusal && <RefusalText refusal={change.refusal} />}
      {change.created && (
        <CopyBlock
          label={`Secret for ${change.created.token.name}`}
          value={change.created.secret}
          data="token-secret"
          valueData={{ 'data-token-secret': true }}
        />
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
  const finished = change?.phase === 'done' || change?.phase === 'refused'
  return (
    <Dialog
      open={open}
      onClose={dismiss}
      data-change-dialog={change?.phase}
      title={change?.title ?? ''}
      description={change ? describe(change) : ''}
      footer={
        finished ? (
          <Button onClick={dismiss}>{change.phase === 'done' ? 'Done' : 'Dismiss'}</Button>
        ) : (
          <>
            <Button type="button" variant="ghost" onClick={dismiss}>
              Cancel
            </Button>
            <Button disabled={change?.phase !== 'preview'} onClick={() => void confirm()}>
              Confirm
            </Button>
          </>
        )
      }
    >
      {change && <Body change={change} />}
    </Dialog>
  )
}
