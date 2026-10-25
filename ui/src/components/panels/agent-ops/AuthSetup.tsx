/**
 * Assistant auth-mode setup (013-agent-sdk-ops, US2/P1): a global (not per-resource) settings
 * form for choosing how the in-app assistant talks to a model — a self-provided Anthropic key
 * (BYOK), a local Ollama endpoint/model, or disabled outright (FR-003, FR-013).
 *
 * Mounted from `AgentPanel.tsx` behind a small gear affordance in its "Ask AI" tab header,
 * rather than a standalone global-settings surface — this app has no such surface yet
 * (searched: no settings dialog/page component exists anywhere under `ui/src/components`), and
 * the assistant's auth mode is the only global setting introduced by this feature so far. If a
 * general settings surface is added later, this form can move there unchanged.
 *
 * Current mode lives in the store's `agentAuth.mode` (`store/handlers/system.ts`'s
 * `AgentAuthModeUpdated` case), loaded on mount via `AgentGetAuthMode` and refreshed after every
 * `AgentSetAuthMode` submit.
 *
 * The stored key itself is never fetched or shown until the operator explicitly clicks "Reveal
 * key" (`AgentRevealKey`) — never on mount, never on mode selection (FR-004, Principle VI). The
 * revealed value lives in `agentAuth.revealedKey` and is cleared locally by "Hide key" (the
 * backend is never asked to forget it — it is only ever removed from view here).
 *
 * All new testids are prefixed `agent-ops-`.
 */
import { useEffect, useState } from 'react'

import { Button, Input } from '@companion/ui'
import type { AgentAuthModeInput } from '@/protocol/types'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

type ModeChoice = 'anthropic' | 'ollama' | 'disabled'

function modeChoiceOf(mode: { type: string } | undefined): ModeChoice {
  switch (mode?.type) {
    case 'Anthropic':
      return 'anthropic'
    case 'Ollama':
      return 'ollama'
    case 'Disabled':
      return 'disabled'
    default:
      return 'anthropic'
  }
}

export function AuthSetup() {
  const request = useStore((s) => s.request)
  const authMode = useStore((s) => s.agentAuth.mode)
  const revealedKey = useStore((s) => s.agentAuth.revealedKey)

  const [choice, setChoice] = useState<ModeChoice>(() => modeChoiceOf(authMode))
  const [key, setKey] = useState('')
  const [endpoint, setEndpoint] = useState('http://localhost:11434')
  const [model, setModel] = useState('llama3.1')
  const [showRevealed, setShowRevealed] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [revealing, setRevealing] = useState(false)
  const [error, setError] = useState<string | undefined>(undefined)
  const [saved, setSaved] = useState(false)

  // Load the current mode once, on mount — mirrors `AgentPanel`'s "send only on mount" contract.
  useEffect(() => {
    request({ type: 'AgentGetAuthMode' }).catch(() => {
      /* best-effort: the form still works, it just starts from the Anthropic default */
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Prefill the form once from the persisted mode, the first time it arrives — never again
  // afterwards, so it never clobbers an in-progress edit. This is React's documented
  // "adjust state during render" pattern (a plain `useState` guard, not a ref, and not a
  // `useEffect` `setState` — see https://react.dev/reference/react/useState#storing-information-from-previous-renders):
  // safe because it is guarded by a comparison and bails out before the second render.
  const [prefilledFrom, setPrefilledFrom] = useState<string | undefined>(undefined)
  if (authMode !== undefined && prefilledFrom !== authMode.type) {
    setPrefilledFrom(authMode.type)
    setChoice(modeChoiceOf(authMode))
    if (authMode.type === 'Ollama') {
      setEndpoint(authMode.data.endpoint)
      setModel(authMode.data.model)
    }
  }

  const submit = () => {
    if (submitting) return
    let mode: AgentAuthModeInput
    if (choice === 'anthropic') {
      mode = { type: 'Anthropic', data: { key } }
    } else if (choice === 'ollama') {
      mode = { type: 'Ollama', data: { endpoint, model } }
    } else {
      mode = { type: 'Disabled' }
    }
    setSubmitting(true)
    setError(undefined)
    setSaved(false)
    void (async () => {
      try {
        const requestId = await request({ type: 'AgentSetAuthMode', data: { mode } })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') {
          setError('The assistant did not respond in time.')
        } else if (!result.ok) {
          setError(result.error ?? 'Saving the assistant auth mode failed.')
        } else {
          setSaved(true)
          if (choice === 'anthropic') setKey('')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setSubmitting(false)
      }
    })()
  }

  const reveal = () => {
    if (revealing) return
    setRevealing(true)
    setError(undefined)
    void (async () => {
      try {
        const requestId = await request({ type: 'AgentRevealKey' })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') {
          setError('The assistant did not respond in time.')
        } else if (!result.ok) {
          setError(result.error ?? 'Revealing the key failed.')
        } else {
          setShowRevealed(true)
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setRevealing(false)
      }
    })()
  }

  return (
    <div className="flex flex-col gap-3 p-3" data-testid="agent-ops-auth-setup">
      <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
        Assistant auth
      </h3>

      <div
        className="flex gap-1"
        data-testid="agent-ops-mode-selector"
        role="radiogroup"
        aria-label="Assistant auth mode"
      >
        <ModeButton
          testId="agent-ops-mode-anthropic"
          label="Anthropic (BYOK)"
          active={choice === 'anthropic'}
          onClick={() => setChoice('anthropic')}
        />
        <ModeButton
          testId="agent-ops-mode-ollama"
          label="Ollama"
          active={choice === 'ollama'}
          onClick={() => setChoice('ollama')}
        />
        <ModeButton
          testId="agent-ops-mode-disabled"
          label="Disabled"
          active={choice === 'disabled'}
          onClick={() => setChoice('disabled')}
        />
      </div>

      {choice === 'anthropic' && (
        <div className="flex flex-col gap-2">
          <div className="flex flex-col gap-1">
            <label htmlFor="agent-ops-byok-key-input" className="text-xs text-muted-foreground">
              API key
            </label>
            <Input
              id="agent-ops-byok-key-input"
              data-testid="agent-ops-byok-key-input"
              type="password"
              autoComplete="off"
              placeholder={
                authMode?.type === 'Anthropic' && authMode.data.key_configured
                  ? 'A key is already configured — enter a new one to replace it'
                  : 'sk-ant-…'
              }
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </div>

          {authMode?.type === 'Anthropic' && authMode.data.key_configured && (
            <div className="flex flex-col gap-1">
              {!showRevealed ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  data-testid="agent-ops-reveal-key"
                  disabled={revealing}
                  onClick={reveal}
                >
                  {revealing ? 'Revealing…' : 'Reveal key'}
                </Button>
              ) : (
                <div className="flex items-center gap-2">
                  <code
                    data-testid="agent-ops-revealed-key"
                    className="flex-1 truncate rounded-sm border border-border bg-card px-2 py-1 font-mono text-xs"
                  >
                    {revealedKey}
                  </code>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    data-testid="agent-ops-hide-key"
                    onClick={() => setShowRevealed(false)}
                  >
                    Hide
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {choice === 'ollama' && (
        <div className="flex flex-col gap-2">
          <div className="flex flex-col gap-1">
            <label
              htmlFor="agent-ops-ollama-endpoint-input"
              className="text-xs text-muted-foreground"
            >
              Endpoint
            </label>
            <Input
              id="agent-ops-ollama-endpoint-input"
              data-testid="agent-ops-ollama-endpoint-input"
              autoComplete="off"
              value={endpoint}
              onChange={(e) => setEndpoint(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor="agent-ops-ollama-model-input" className="text-xs text-muted-foreground">
              Model
            </label>
            <Input
              id="agent-ops-ollama-model-input"
              data-testid="agent-ops-ollama-model-input"
              autoComplete="off"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </div>
        </div>
      )}

      {error && (
        <p data-testid="agent-ops-auth-error" className="text-xs text-status-err">
          {error}
        </p>
      )}
      {saved && !error && (
        <p data-testid="agent-ops-auth-saved" className="text-xs text-status-ok">
          Saved.
        </p>
      )}

      <div>
        <Button
          type="button"
          size="sm"
          data-testid="agent-ops-auth-submit"
          disabled={submitting || (choice === 'anthropic' && key.trim().length === 0)}
          onClick={submit}
        >
          {submitting ? 'Saving…' : 'Save'}
        </Button>
      </div>
    </div>
  )
}

function ModeButton({
  testId,
  label,
  active,
  onClick,
}: {
  testId: string
  label: string
  active: boolean
  onClick: () => void
}) {
  return (
    <Button
      type="button"
      variant={active ? 'default' : 'outline'}
      size="sm"
      data-testid={testId}
      role="radio"
      aria-checked={active}
      onClick={onClick}
    >
      {label}
    </Button>
  )
}
