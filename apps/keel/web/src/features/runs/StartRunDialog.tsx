/**
 * Start a job by hand (spec 068 T051, T056, T066; contracts/ui.md "Start dialog"): the job's
 * params (an image digest, or a `choice` listed at the binding's current commit), a Check toggle
 * for a dry run, and the credentials the run will be sent. Start hands `PreviewRunStart` to
 * governance, whose change dialog shows the server's preview and confirms it.
 */
import { PlayIcon } from '@phosphor-icons/react'
import { Button, Input, Switch } from '@companion/ui'
import { Dialog, Field, Select } from '@companion/ui/kit'
import { useId, useState } from 'react'

import { useConverge } from '@/features/converge/store'
import { useGovernance } from '@/features/teams/store'
import type { JobInfo, ParamView } from '@/protocol/wire'

import { initialParams, paramNames, paramProblem, paramsReady, paramsToSend } from './run-068'

function ParamField({
  name,
  view,
  value,
  shown,
  onChange,
}: {
  name: string
  view: ParamView
  value: string
  shown: boolean
  onChange: (v: string) => void
}) {
  const id = useId()
  const problem = paramProblem(view, value)
  const stuck = view.kind === 'choice' && (view.unavailable !== null || (view.allowed ?? []).length === 0)
  // A choice that can't be listed says so at once; a value problem waits for a Start attempt.
  const hint = stuck || (shown && problem) ? problem : view.kind === 'oci_digest' ? 'sha256: and 64 hex characters' : null
  const hintId = `${id}-hint`
  return (
    <Field label={<span className="font-mono">{name}</span>} htmlFor={id}>
      {view.kind === 'choice' ? (
        <Select
          id={id}
          data-param={name}
          value={value}
          disabled={stuck}
          aria-invalid={shown && problem !== null}
          aria-describedby={hint ? hintId : undefined}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        >
          <option value="">{stuck ? 'Nothing to pick' : 'Pick a value…'}</option>
          {(view.allowed ?? []).map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </Select>
      ) : (
        <Input
          id={id}
          data-param={name}
          value={value}
          spellCheck={false}
          autoComplete="off"
          className="font-mono"
          placeholder="sha256:…"
          aria-invalid={shown && problem !== null}
          aria-describedby={hint ? hintId : undefined}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )}
      {hint && (
        <p id={hintId} className={`text-xs ${stuck || shown ? 'text-status-err' : 'text-muted-foreground'}`}>
          {hint}
        </p>
      )}
    </Field>
  )
}

function Form({ job, onClose }: { job: JobInfo; onClose: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const convergeOff = useConverge((s) => s.disabled)
  const [values, setValues] = useState<Record<string, string>>(() => initialParams(job.params))
  const [check, setCheck] = useState(false)
  const [tried, setTried] = useState(false)
  const checkId = useId()
  const names = paramNames(job.params)

  const start = () => {
    if (!paramsReady(job.params, values)) {
      setTried(true)
      return
    }
    onClose()
    propose(`${check ? 'Check' : 'Run'} ${job.name}`, {
      type: 'PreviewRunStart',
      data: { job: job.id, limit: null, params: paramsToSend(job.params, values), check },
    })
  }

  return (
    <form
      id="start-run"
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        start()
      }}
    >
      {names.length > 0 && (
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-xs font-medium">Params</legend>
          {names.map((n) => {
            const view = job.params?.[n]
            if (!view) return null
            return (
              <ParamField
                key={n}
                name={n}
                view={view}
                value={values[n] ?? ''}
                shown={tried}
                onChange={(v) => {
                  setValues((s) => ({ ...s, [n]: v }))
                }}
              />
            )
          })}
        </fieldset>
      )}
      {!convergeOff && (
        <div className="flex items-start gap-2 text-xs">
          <Switch
            id={checkId}
            data-check-toggle
            checked={check}
            onCheckedChange={(on) => {
              setCheck(on)
            }}
          />
          <label htmlFor={checkId} className="flex flex-col">
            <span className="font-medium">Check only</span>
            <span className="text-muted-foreground">
              Runs <code className="font-mono">ansible-playbook --check --diff</code>: shows what would change, changes
              nothing.
            </span>
          </label>
        </div>
      )}
      <p data-credentials className="text-xs break-words text-muted-foreground">
        {job.credentials.length === 0 ? (
          'No credentials are sent to the hosts.'
        ) : (
          <>
            Credentials sent: <span className="font-mono text-foreground">{job.credentials.join(', ')}</span>
          </>
        )}
      </p>
    </form>
  )
}

/** The Start dialog for `job`; `null` closes it. */
export function StartRunDialog({ job, onClose }: { job: JobInfo | null; onClose: () => void }) {
  return (
    <Dialog
      open={job !== null}
      onClose={onClose}
      title={job ? `Run ${job.name}` : 'Run'}
      description="You review the exact change before anything is sent."
      data-start-run
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" form="start-run">
            <PlayIcon aria-hidden />
            Review run
          </Button>
        </>
      }
    >
      {job && <Form key={job.id} job={job} onClose={onClose} />}
    </Dialog>
  )
}
