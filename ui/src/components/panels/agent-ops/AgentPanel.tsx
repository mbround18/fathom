/**
 * "Agent" tab (013-agent-sdk-ops, US1/P1 MVP): a per-resource conversation with the in-app
 * troubleshooting assistant. Opens (or loads) the conversation for this object on mount via
 * `AgentOpenConversation`, then lets the operator ask questions via `AgentAsk`.
 *
 * Conversation state lives in the store's `agentConversations` slice
 * (`store/handlers/agent.ts`), keyed by `${kind}/${namespace}/${name}` — driven entirely by
 * `AgentConversationUpdated` events, never written locally.
 *
 * `SuggestedAction`s attached to an assistant message are rendered here as plain, read-only
 * cards (rationale + action kind + target) — Phase 5 (US3) is what makes these interactive
 * (`SuggestedActionCard.tsx`'s Review/Decline flow through the app's existing confirm-dialog
 * path, constitution Principle I). Nothing here is clickable/actionable yet.
 *
 * All new testids introduced by this feature are prefixed `agent-ops-`.
 */
import { useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import type { AgentMessage, AgentObjectRef } from '@/protocol/types'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'
import { agentConversationKey } from '@/store/handlers/agent'

import { AuthSetup } from './AuthSetup'

export function AgentPanel({ object }: { object: AgentObjectRef }) {
  const request = useStore((s) => s.request)
  const key = agentConversationKey(object.kind, object.namespace, object.name)
  const conversation = useStore((s) => s.agentConversations[key])
  const messages = conversation?.messages ?? []

  const [question, setQuestion] = useState('')
  const [asking, setAsking] = useState(false)
  const [error, setError] = useState<string | undefined>(undefined)
  // 013-agent-sdk-ops (US2/P1): the assistant's auth mode is a global setting, not per-object —
  // this app has no standalone global-settings surface yet, so `AuthSetup` is mounted here,
  // behind an explicit toggle, rather than always shown above the conversation.
  const [showAuthSetup, setShowAuthSetup] = useState(false)

  // Load (or create) this object's conversation once per object, on mount — mirrors
  // `ImageInspectPanel`'s "send only on mount/target-change" contract.
  const openedFor = useRef<string | undefined>(undefined)
  useEffect(() => {
    if (openedFor.current === key) return
    openedFor.current = key
    // Best-effort: a transport-level failure here (e.g. not connected yet) is not fatal — the
    // operator can still type a question, and `AgentAsk`'s own error surfaces through `submit`.
    request({ type: 'AgentOpenConversation', data: { object } }).catch(() => {
      /* no-op: see comment above */
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  const submit = () => {
    const trimmed = question.trim()
    if (!trimmed || asking) return
    setAsking(true)
    setError(undefined)
    setQuestion('')
    void (async () => {
      try {
        const requestId = await request({ type: 'AgentAsk', data: { object, question: trimmed } })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') {
          setError('The assistant did not respond in time.')
        } else if (!result.ok) {
          setError(result.error ?? 'The assistant request failed.')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setAsking(false)
      }
    })()
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-3">
      <div className="flex shrink-0 justify-end">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-testid="agent-ops-auth-toggle"
          aria-expanded={showAuthSetup}
          onClick={() => setShowAuthSetup((v) => !v)}
        >
          {showAuthSetup ? 'Close auth settings' : 'Auth settings'}
        </Button>
      </div>
      {showAuthSetup && (
        <div className="shrink-0 border-b border-border pb-3">
          <AuthSetup />
        </div>
      )}
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto" data-testid="agent-ops-messages">
        {messages.length === 0 && !asking && (
          <p data-testid="agent-ops-empty" className="text-xs text-muted-foreground">
            Ask a question about {object.kind.toLowerCase()} {object.namespace}/{object.name} to
            get started.
          </p>
        )}
        {messages.map((m) => (
          <MessageBubble key={m.id} message={m} />
        ))}
        {asking && (
          <p
            data-testid="agent-ops-loading"
            className="text-xs text-muted-foreground"
            role="status"
          >
            Thinking…
          </p>
        )}
        {error && (
          <p data-testid="agent-ops-error" className="text-xs text-status-err">
            {error}
          </p>
        )}
      </div>

      <div className="flex shrink-0 flex-col gap-2 border-t border-border pt-3">
        <textarea
          data-testid="agent-ops-question-input"
          className="min-h-16 w-full resize-none rounded-md border border-border bg-background p-2 font-mono text-xs"
          placeholder="Ask about this resource…"
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              submit()
            }
          }}
          disabled={asking}
        />
        <div>
          <Button
            type="button"
            size="sm"
            data-testid="agent-ops-submit"
            disabled={asking || question.trim().length === 0}
            onClick={submit}
          >
            {asking ? 'Asking…' : 'Ask'}
          </Button>
        </div>
      </div>
    </div>
  )
}

function MessageBubble({ message }: { message: AgentMessage }) {
  const isOperator = message.author === 'Operator'
  return (
    <div
      data-testid="agent-ops-message"
      data-author={message.author}
      className={`flex flex-col gap-1 rounded-md border border-border p-2 text-xs ${
        isOperator ? 'bg-muted/30' : 'bg-background'
      }`}
    >
      <span className="font-display text-[10px] tracking-wider text-muted-foreground uppercase">
        {isOperator ? 'You' : 'Assistant'}
      </span>
      <p className="whitespace-pre-wrap text-foreground/90">{message.text}</p>
      {message.suggested_actions.length > 0 && (
        <div className="flex flex-col gap-2 pt-1">
          {message.suggested_actions.map((sa) => (
            // Phase 5 (US3) makes this card interactive (Review/Decline via
            // `SuggestedActionCard.tsx`): plain, read-only display only, for now.
            <div
              key={sa.id}
              data-testid="agent-ops-suggested-action"
              className="rounded-md border border-accent-neon/40 bg-accent-neon/5 p-2"
            >
              <p className="font-display text-[10px] tracking-wider text-accent-neon uppercase">
                {sa.action_kind} · {sa.target.kind} {sa.target.namespace}/{sa.target.name}
              </p>
              <p className="pt-1 text-foreground/80">{sa.rationale}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
