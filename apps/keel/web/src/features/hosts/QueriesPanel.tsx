/**
 * The host inspector's Queries panel (spec 067 US5, T043; contracts/ui.md "Hosts"): pick a kind,
 * fill its one parameter if it takes one, Ask. Answers list newest first with time, asker and
 * outcome; an answer opens to its JSON. A runner without `query-v1` shows "runner too old for
 * queries" instead of the form. Only operators ask; viewers see the list.
 */
import { QuestionIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { LABEL, RefusalText, Select } from '@companion/ui/kit'
import { useEffect, useId, useState } from 'react'

import { useSession } from '@/app/session-store'
import { when } from '@/features/settings/updates'
import type { HostInfo, HostQuery, QueryKind } from '@/protocol/wire'

import { answerText, askedText, KIND_LABEL, KINDS, outcomeText, outcomeTone, paramOf, paramProblem, type OutcomeTone } from './queries'
import { useQueries } from './queries-store'
import { canOperate } from './workspaces'

const TONE: Record<OutcomeTone, string> = {
  ok: 'text-status-ok',
  warn: 'text-status-warn',
  err: 'text-status-err',
  quiet: 'text-muted-foreground',
}

const PLACEHOLDER = { unit: 'nginx.service', path: '/var/lib' } as const

function AskForm({ host }: { host: HostInfo }) {
  const ask = useQueries((s) => s.ask)
  const asking = useQueries((s) => s.asking)
  const askRefusal = useQueries((s) => s.askRefusal)
  const [kind, setKind] = useState<QueryKind>('unit_state')
  const [value, setValue] = useState('')
  const [touched, setTouched] = useState(false)
  const kindId = useId()
  const paramId = useId()
  const problemId = useId()
  const param = paramOf(kind)
  const problem = paramProblem(kind, value.trim())
  return (
    <form
      aria-label="Ask the host"
      className="flex min-w-0 flex-col gap-1.5"
      onSubmit={(e) => {
        e.preventDefault()
        setTouched(true)
        if (problem !== null) return
        ask(kind, value.trim())
      }}
    >
      <div className="flex min-w-0 flex-wrap items-end gap-2">
        <label htmlFor={kindId} className="flex min-w-0 flex-col gap-0.5">
          <span className="text-muted-foreground">Kind</span>
          <Select
            id={kindId}
            dense
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as QueryKind)
              setTouched(false)
            }}
          >
            {KINDS.map((k) => (
              <option key={k} value={k}>
                {KIND_LABEL[k]}
              </option>
            ))}
          </Select>
        </label>
        {param !== null && (
          <label htmlFor={paramId} className="flex min-w-0 flex-1 basis-40 flex-col gap-0.5">
            <span className="text-muted-foreground">{param === 'unit' ? 'Unit' : 'Path'}</span>
            <Input
              id={paramId}
              className="h-8 font-mono text-xs"
              value={value}
              placeholder={PLACEHOLDER[param]}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={touched && problem !== null}
              aria-describedby={touched && problem !== null ? problemId : undefined}
              onChange={(e) => {
                setValue(e.target.value)
              }}
            />
          </label>
        )}
        <Button type="submit" size="sm" variant="outline" disabled={asking || host.state === 'revoked'}>
          <QuestionIcon aria-hidden />
          {asking ? 'Asking…' : 'Ask'}
        </Button>
      </div>
      {touched && problem !== null && (
        <p id={problemId} className="text-status-err">
          {problem}
        </p>
      )}
      {askRefusal && <RefusalText refusal={askRefusal} />}
    </form>
  )
}

function QueryRow({ q }: { q: HostQuery }) {
  const tone = outcomeTone(q.outcome)
  return (
    <li data-query-outcome={q.outcome ?? 'waiting'} className="flex min-w-0 flex-col gap-0.5 border-t border-border px-2 py-1.5 first:border-t-0">
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
        <span className="min-w-0 font-mono break-all">{askedText(q)}</span>
        <span className={`font-medium ${TONE[tone]}`}>{outcomeText(q)}</span>
        <span className="ml-auto flex flex-wrap gap-x-3 text-muted-foreground">
          <span>{q.asked_by}</span>
          <time dateTime={q.asked_at} className="tabular-nums">
            {when(q.asked_at)}
          </time>
        </span>
      </div>
      {q.answer !== null && (
        <details>
          <summary className="cursor-pointer text-muted-foreground">Answer</summary>
          <pre className="mt-1 max-h-64 overflow-auto border border-border bg-muted/20 p-2 font-mono text-[11px] whitespace-pre-wrap break-all">
            {answerText(q.answer)}
          </pre>
        </details>
      )}
    </li>
  )
}

/** In the host's panel: ask it a typed question and read the answers. */
export function QueriesPanel({ host }: { host: HostInfo }) {
  const me = useSession((s) => s.me)
  const load = useQueries((s) => s.load)
  const items = useQueries((s) => (s.host === host.id ? s.items : null))
  const queryV1 = useQueries((s) => (s.host === host.id ? s.queryV1 : null))
  const refusal = useQueries((s) => (s.host === host.id ? s.refusal : null))
  useEffect(() => {
    load(host.id)
  }, [host.id, load])
  const operator = canOperate(me, host.workspace)
  return (
    <section aria-label="Queries" className="flex min-w-0 flex-col gap-1.5">
      <h3 className={LABEL}>Queries</h3>
      {queryV1 === false ? (
        <p data-queries="too-old" className="text-status-warn">
          Runner too old for queries. Upgrade keel-runner on this host to ask it.
        </p>
      ) : (
        <>
          {queryV1 === null && items !== null && (
            <p data-queries="offline" className="text-muted-foreground">
              The host is not connected, so a question now is answered “host not connected”.
            </p>
          )}
          {operator && <AskForm host={host} />}
        </>
      )}
      {refusal && <RefusalText refusal={refusal} />}
      {items !== null &&
        (items.length === 0 ? (
          <p data-pane-state="empty" className="text-muted-foreground">
            Nothing asked yet.
          </p>
        ) : (
          <ol aria-label="Asked" className="flex min-w-0 flex-col border border-border">
            {items.map((q) => (
              <QueryRow key={q.id} q={q} />
            ))}
          </ol>
        ))}
    </section>
  )
}
