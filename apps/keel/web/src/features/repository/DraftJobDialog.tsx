/**
 * Draft job (spec 066 T052, contracts/ui.md "Playbook detail"): the `.keel/jobs.yaml` entry Keel
 * derives for a playbook (`DraftJob`, R6), read-only with Copy. With a forge connection it also
 * offers Propose as pull request: a job name, then the governance preview and confirm
 * (`PreviewJobPropose`), then the outcome. While the change dialog is up this one steps aside, so
 * only one dialog holds focus, and it comes back with the outcome.
 */
import { GitPullRequestIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { CopyBlock, Dialog, Field, LoadFailed, LoadState } from '@companion/ui/kit'
import { useEffect, useId, useState } from 'react'

import { loadList, type Refusal } from '@/app/load'
import { useForge } from '@/features/settings/forge-store'
import { useGovernance } from '@/features/teams/store'
import type { ChangeOutcome } from '@/protocol/wire'

import { playbookStem } from './catalog'
import { ChangeOutcomeView } from './ChangeOutcomeView'

/** What `.keel/jobs.yaml` accepts as a job name. */
const JOB_NAME = /^[a-z0-9][a-z0-9_-]{0,62}$/

type Draft = { yaml: string; notes: string[] }

/** What `DraftJob` answered for one request, by its key (playbook and attempt). */
type Answer = { key: string; draft: Draft | null; failed: Refusal | null }

function useDraft(workspace: string, playbook: string | null): { draft: Draft | null; failed: Refusal | null; retry: () => void } {
  const [answer, setAnswer] = useState<Answer | null>(null)
  const [attempt, setAttempt] = useState(0)
  const key = `${workspace} ${playbook ?? ''} ${String(attempt)}`
  useEffect(() => {
    if (playbook === null) return
    let live = true
    loadList({ type: 'DraftJob', data: { workspace, playbook } }, (e) => (e.type === 'JobDraft' ? e.data : undefined), {
      loaded: (draft) => {
        if (live) setAnswer({ key, draft, failed: null })
      },
      failed: (failed) => {
        if (live) setAnswer({ key, draft: null, failed })
      },
    })
    return () => {
      live = false
    }
  }, [workspace, playbook, key])
  // An answer for an earlier playbook or attempt is not this one's.
  const current = answer?.key === key ? answer : null
  return {
    draft: current?.draft ?? null,
    failed: current?.failed ?? null,
    retry: () => {
      setAttempt((a) => a + 1)
    },
  }
}

export function DraftJobDialog({ workspace, playbook, onClose }: { workspace: string; playbook: string | null; onClose: () => void }) {
  const { draft, failed, retry } = useDraft(workspace, playbook)
  const connection = useForge((s) => s.connection)
  const loadForge = useForge((s) => s.load)
  const propose = useGovernance((s) => s.propose)
  const changing = useGovernance((s) => s.change !== null)
  const [name, setName] = useState(playbook === null ? '' : playbookStem(playbook))
  const [outcome, setOutcome] = useState<ChangeOutcome | null>(null)
  // A new playbook starts a new draft: reset while rendering, not in an effect.
  const [shown, setShown] = useState(playbook)
  if (shown !== playbook) {
    setShown(playbook)
    setName(playbook === null ? '' : playbookStem(playbook))
    setOutcome(null)
  }
  const nameId = useId()

  useEffect(() => {
    if (playbook !== null) loadForge(workspace)
  }, [playbook, workspace, loadForge])

  const valid = JOB_NAME.test(name)
  const submit = () => {
    if (playbook === null || !valid) return
    propose('Propose job', { type: 'PreviewJobPropose', data: { workspace, playbook, name } }, (event) => {
      if (event.type === 'ChangeProposed') setOutcome(event.data)
    })
  }

  return (
    <Dialog
      open={playbook !== null && !changing}
      onClose={onClose}
      size="lg"
      data-draft-dialog
      title="Draft job"
      description={playbook === null ? '' : `A .keel/jobs.yaml entry for ${playbook}. Check the profiles and paths before you use it.`}
      footer={<Button onClick={onClose}>{outcome ? 'Done' : 'Close'}</Button>}
    >
      {failed && <LoadFailed what="the draft" refusal={failed} onRetry={retry} />}
      {!failed && !draft && <LoadState loading label="Drafting…" />}
      {draft && (
        <div className="flex min-w-0 flex-col gap-3">
          <CopyBlock label="Job entry" value={draft.yaml} data="draft-yaml" valueData={{ 'data-testid': 'draft-yaml' }} />
          {draft.notes.length > 0 && (
            <ul aria-label="Notes" className="flex min-w-0 list-disc flex-col gap-0.5 pl-4 text-xs text-muted-foreground">
              {draft.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}
          {outcome ? (
            <ChangeOutcomeView outcome={outcome} />
          ) : connection ? (
            <form
              className="flex min-w-0 flex-wrap items-end gap-2"
              onSubmit={(e) => {
                e.preventDefault()
                submit()
              }}
            >
              <Field label="Job name" htmlFor={nameId} className="min-w-40 flex-1" hint={valid ? undefined : 'Lower case letters, digits, - and _.'}>
                <Input
                  id={nameId}
                  value={name}
                  aria-invalid={!valid}
                  onChange={(e) => {
                    setName(e.target.value)
                  }}
                />
              </Field>
              <Button type="submit" size="sm" disabled={!valid}>
                <GitPullRequestIcon aria-hidden />
                Propose as pull request
              </Button>
            </form>
          ) : (
            connection === null && (
              <p data-no-forge className="text-xs text-muted-foreground">
                Copy the entry into .keel/jobs.yaml. With a forge connection in Settings → Workspace, Keel can open the pull request for you.
              </p>
            )
          )}
        </div>
      )}
    </Dialog>
  )
}
