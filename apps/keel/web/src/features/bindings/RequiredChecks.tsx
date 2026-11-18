/**
 * Required checks on a binding (spec 068 US7 T060, contracts/ui.md "Binding page"): the forge
 * commit-status contexts a run from this binding waits for, and how long it waits. Admins add and
 * remove contexts and change the timeout; each change is a `PreviewRequiredChecks` confirmed with a
 * passkey (step-up) in the shared preview dialog. A forge that can't report statuses refuses with
 * `checks_unsupported`, and the section then says so and stops offering to add.
 */
import { PlusIcon, XIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { LABEL } from '@companion/ui/kit'
import { useId, useState, type SyntheticEvent } from 'react'

import { contextProblem, MAX_TIMEOUT_MIN, MIN_TIMEOUT_MIN, timeoutText } from '@/features/runs/run-068'
import type { BindingInfo } from '@/protocol/wire'

import { useRepos } from './store'

/** Refusals this section answers inline rather than only in the page's error line. */
const OURS = new Set(['checks_unsupported', 'invalid_context', 'too_many_contexts', 'invalid_timeout', 'binding_not_synced', 'no_change'])

export function RequiredChecks({ binding, canAct }: { binding: BindingInfo; canAct: boolean }) {
  const propose = useRepos((s) => s.propose)
  const error = useRepos((s) => s.error)
  const code = useRepos((s) => s.errorCode)
  const [asked, setAsked] = useState(false)
  const [context, setContext] = useState('')
  const [tried, setTried] = useState(false)
  const [minutes, setMinutes] = useState(String(Math.round(binding.checks_timeout_secs / 60)))
  const id = useId()
  const contexts = binding.required_checks.map((c) => c.context)
  const problem = contextProblem(context, contexts)
  const refusal = asked && code !== null && OURS.has(code) ? error : null
  const unsupported = asked && code === 'checks_unsupported'
  const mins = Number(minutes)
  const minutesOk = Number.isInteger(mins) && mins >= MIN_TIMEOUT_MIN && mins <= MAX_TIMEOUT_MIN
  const timeoutChanged = minutesOk && mins * 60 !== binding.checks_timeout_secs

  const ask = (next: string[], timeout: number | null) => {
    setAsked(true)
    propose({
      type: 'PreviewRequiredChecks',
      data: { binding_id: binding.id, contexts: next, ...(timeout === null ? {} : { timeout_secs: timeout }) },
    })
  }

  const add = (e: SyntheticEvent) => {
    e.preventDefault()
    if (problem) {
      setTried(true)
      return
    }
    setTried(false)
    ask([...contexts, context.trim()], null)
    setContext('')
  }

  return (
    <section aria-labelledby={`${id}-title`} data-required-checks className="flex min-w-0 flex-col gap-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h3 id={`${id}-title`} className={LABEL}>
          Required checks
        </h3>
        <span className="text-xs text-muted-foreground">
          {contexts.length === 0
            ? 'None: runs start without waiting on the forge.'
            : `Runs wait up to ${timeoutText(binding.checks_timeout_secs)} for these to pass on their commit.`}
        </span>
      </div>
      {contexts.length > 0 && (
        <ul aria-label="Required checks" className="flex min-w-0 flex-wrap gap-1">
          {contexts.map((c) => (
            <li key={c} data-required-check={c} className="flex min-w-0 items-center gap-1 border border-border py-0.5 pr-0.5 pl-2 text-xs">
              <span className="min-w-0 font-mono break-all">{c}</span>
              {canAct && (
                <Button
                  size="xs"
                  variant="ghost"
                  aria-label={`Stop requiring ${c}`}
                  onClick={() => {
                    ask(
                      contexts.filter((x) => x !== c),
                      null,
                    )
                  }}
                >
                  <XIcon aria-hidden />
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
      {refusal && (
        <p role="alert" data-checks-refusal={code} className="text-xs break-words text-status-err">
          {refusal}
        </p>
      )}
      {canAct && (
        <div className="flex min-w-0 flex-wrap items-start gap-x-4 gap-y-2">
          <form aria-label="Require a check" onSubmit={add} className="flex min-w-0 flex-col gap-1">
            <label htmlFor={`${id}-context`} className={LABEL}>
              Status context
            </label>
            <div className="flex min-w-0 gap-1">
              <Input
                id={`${id}-context`}
                value={context}
                disabled={unsupported}
                placeholder="ci/build"
                spellCheck={false}
                autoComplete="off"
                className="w-48 min-w-0 font-mono"
                aria-invalid={tried && problem !== null}
                aria-describedby={tried && problem ? `${id}-context-problem` : undefined}
                onChange={(e) => {
                  setContext(e.target.value)
                }}
              />
              <Button type="submit" size="sm" variant="outline" disabled={unsupported}>
                <PlusIcon aria-hidden />
                Require
              </Button>
            </div>
            {tried && problem && (
              <p id={`${id}-context-problem`} className="text-xs text-status-err">
                {problem}
              </p>
            )}
          </form>
          {contexts.length > 0 && (
            <form
              aria-label="Check timeout"
              className="flex min-w-0 flex-col gap-1"
              onSubmit={(e) => {
                e.preventDefault()
                if (timeoutChanged) ask(contexts, mins * 60)
              }}
            >
              <label htmlFor={`${id}-timeout`} className={LABEL}>
                Wait at most (minutes)
              </label>
              <div className="flex min-w-0 gap-1">
                <Input
                  id={`${id}-timeout`}
                  type="number"
                  inputMode="numeric"
                  min={MIN_TIMEOUT_MIN}
                  max={MAX_TIMEOUT_MIN}
                  step={1}
                  value={minutes}
                  className="w-24 tabular-nums"
                  aria-invalid={!minutesOk}
                  onChange={(e) => {
                    setMinutes(e.target.value)
                  }}
                />
                <Button type="submit" size="sm" variant="outline" disabled={!timeoutChanged}>
                  Save
                </Button>
              </div>
              {!minutesOk && (
                <p className="text-xs text-status-err">
                  {String(MIN_TIMEOUT_MIN)} to {String(MAX_TIMEOUT_MIN)} minutes.
                </p>
              )}
            </form>
          )}
        </div>
      )}
    </section>
  )
}
