/**
 * "Agent" tab (013-agent-sdk-ops, US1/P1 MVP): a per-resource conversation with the in-app
 * troubleshooting assistant. Opens (or loads) the conversation for this object on mount via
 * `AgentOpenConversation`, then lets the operator ask questions via `AgentAsk`.
 *
 * Conversation state lives in the store's `agentConversations` slice
 * (`store/handlers/agent.ts`), keyed by `${kind}/${namespace}/${name}` — driven entirely by
 * `AgentConversationUpdated` events, never written locally.
 *
 * `SuggestedAction`s attached to an assistant message render as `SuggestedActionCard`s (Phase
 * 5/US3): Review opens the app's existing `ConfirmDialog` with a diff computed fresh at click
 * time, and confirming it runs the same unmodified mutating command a manual click would,
 * through the same `runAction` — see `reviewSuggestedAction.ts` (constitution Principle I).
 *
 * All new testids introduced by this feature are prefixed `agent-ops-`.
 */
import { useEffect, useRef, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@companion/ui'
import {
  DISCLOSURE_TEXT,
  DISCLOSURE_TITLE,
  needsHostedDisclosure,
} from '@/features/ai-assist/disclosure'
import type {
  AgentMessage,
  AgentObjectRef,
  ExecutedAction,
  GroundingSource,
} from '@/protocol/types'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'
import { agentConversationKey } from '@/store/handlers/agent'

import { AuthSetup } from './AuthSetup'
import { SuggestedActionCard } from './SuggestedActionCard'

export function AgentPanel({ object }: { object: AgentObjectRef }) {
  const request = useStore((s) => s.request)
  const key = agentConversationKey(object.kind, object.namespace, object.name)
  const conversation = useStore((s) => s.agentConversations[key])
  const messages = conversation?.messages ?? []
  // FR-003/FR-013: with no assistant configured, or the assistant disabled, no question can be
  // submitted — the panel shows a setup prompt instead. Known only once the mode has loaded; a
  // provider the 013 mode cannot represent (038's GitHub/OpenAI) still counts as configured.
  const blocked = useStore(
    (s) =>
      (s.agentAuth.mode?.type === 'Unconfigured' || s.agentAuth.mode?.type === 'Disabled') &&
      !(s.ai.loaded && s.ai.active !== null),
  )
  // Hosted providers receive this resource's logs and events with the question: the operator
  // is told so once, before the first request, and the acceptance is persisted in `UiConfig`.
  // Local Ollama never prompts (`features/ai-assist/disclosure.ts`).
  const needsDisclosure = useStore((s) =>
    needsHostedDisclosure({
      active: s.ai.active,
      legacyMode: s.agentAuth.mode,
      accepted: s.ui.aiDisclosureAccepted,
    }),
  )
  const acceptAiDisclosure = useStore((s) => s.acceptAiDisclosure)
  const [confirmingSend, setConfirmingSend] = useState(false)

  const [question, setQuestion] = useState('')
  const [asking, setAsking] = useState(false)
  const [error, setError] = useState<string | undefined>(undefined)
  // 013-agent-sdk-ops (US2/P1): the assistant's auth mode is a global setting, not per-object —
  // this app has no standalone global-settings surface yet, so `AuthSetup` is mounted here,
  // behind an explicit toggle, rather than always shown above the conversation.
  const [showAuthSetup, setShowAuthSetup] = useState(false)
  // 013-agent-sdk-ops (US4/P3): "Clear history" is a local-only, non-cluster-mutating action —
  // it never touches the cluster — but a misclick would permanently wipe this object's
  // conversation (FR-017), so it is still confirmation-gated via the app's existing
  // `ConfirmDialog`, reused with a single item representing this object.
  const [confirmingClear, setConfirmingClear] = useState(false)

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
    if (!trimmed || asking || blocked) return
    if (needsDisclosure) {
      // Nothing is sent until the operator confirms; the question stays in the box on cancel.
      setConfirmingSend(true)
      return
    }
    send(trimmed)
  }

  const send = (trimmed: string) => {
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
      <div className="flex shrink-0 justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-testid="agent-ops-clear-history"
          disabled={messages.length === 0}
          onClick={() => setConfirmingClear(true)}
        >
          Clear history
        </Button>
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
      {confirmingClear && (
        <ConfirmDialog
          open
          title="Clear conversation history?"
          items={[{ namespace: object.namespace, name: object.name }]}
          destructive
          confirmLabel="Clear history"
          onConfirm={() => {
            setConfirmingClear(false)
            void request({ type: 'AgentClearConversation', data: { object } }).catch(() => {
              /* no-op: AgentConversationUpdated (or its absence) is the source of truth */
            })
          }}
          onCancel={() => setConfirmingClear(false)}
        />
      )}
      {confirmingSend && (
        <ConfirmDialog
          open
          title={DISCLOSURE_TITLE}
          items={[{ namespace: object.namespace, name: object.name }]}
          requireTypedCount={false}
          detail={<p data-testid="agent-ops-disclosure">{DISCLOSURE_TEXT}</p>}
          confirmLabel="Send"
          onConfirm={() => {
            setConfirmingSend(false)
            acceptAiDisclosure()
            send(question.trim())
          }}
          onCancel={() => setConfirmingSend(false)}
        />
      )}
      {blocked && !showAuthSetup && (
        <div
          data-testid="agent-ops-setup-required"
          className="flex shrink-0 flex-col items-start gap-2 rounded-md border border-border p-2 text-xs"
        >
          <p className="text-muted-foreground">
            The assistant is disabled or not set up. Enter an Anthropic API key or use a local
            Ollama model to ask questions.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="agent-ops-setup-open"
            onClick={() => setShowAuthSetup(true)}
          >
            Set up assistant
          </Button>
        </div>
      )}
      {showAuthSetup && (
        <div className="shrink-0 border-b border-border pb-3">
          <AuthSetup />
        </div>
      )}
      <div
        className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto"
        data-testid="agent-ops-messages"
      >
        {messages.length === 0 && !asking && (
          <p data-testid="agent-ops-empty" className="text-xs text-muted-foreground">
            Ask a question about {object.kind.toLowerCase()} {object.namespace}/{object.name} to get
            started.
          </p>
        )}
        {messages.map((m) => (
          <MessageBubble
            key={m.id}
            object={object}
            message={m}
            executedActions={conversation?.executed_actions ?? []}
          />
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
          disabled={asking || blocked}
        />
        <div>
          <Button
            type="button"
            size="sm"
            data-testid="agent-ops-submit"
            disabled={asking || blocked || question.trim().length === 0}
            onClick={submit}
          >
            {asking ? 'Asking…' : 'Ask'}
          </Button>
        </div>
      </div>
    </div>
  )
}

function MessageBubble({
  object,
  message,
  executedActions,
}: {
  object: AgentObjectRef
  message: AgentMessage
  executedActions: readonly ExecutedAction[]
}) {
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
      {message.grounding.length > 0 && (
        <ul
          data-testid="agent-ops-grounding-sources"
          aria-label="Grounded in"
          className="flex flex-wrap gap-1"
        >
          {message.grounding.map((g, i) => (
            <li
              key={i}
              data-testid="agent-ops-grounding-source"
              data-source={g.type}
              className="rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground"
            >
              {groundingLabel(g)}
            </li>
          ))}
        </ul>
      )}
      {message.suggested_actions.length > 0 && (
        <div className="flex flex-col gap-2 pt-1">
          {message.suggested_actions.map((sa) => (
            <SuggestedActionCard
              key={sa.id}
              object={object}
              suggestion={sa}
              executedOutcome={
                executedActions.find((e) => e.suggested_action_id === sa.id)?.outcome
              }
            />
          ))}
        </div>
      )}
    </div>
  )
}

/** FR-012: which data grounded an answer; FR-018: a truncated log tail says so. */
function groundingLabel(g: GroundingSource): string {
  switch (g.type) {
    case 'Events':
      return 'Events'
    case 'Conditions':
      return 'Conditions'
    case 'ContainerStatuses':
      return 'Container statuses'
    case 'Logs':
      return g.data.truncated ? 'Logs (truncated)' : 'Logs'
    case 'Lifecycle':
      return 'Lifecycle'
  }
}
