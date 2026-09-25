import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { LoginFlow, Provider, ProviderStatus } from '@/protocol/types'
import { useStore } from '@/store'

import { PROVIDER_LABEL, stateLabel } from './labels'

function useCountdown(expiresAt: string | null | undefined): number | undefined {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!expiresAt) return
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [expiresAt])
  if (!expiresAt) return undefined
  return Math.max(0, Math.round((Date.parse(expiresAt) - now) / 1000))
}

function DeviceCode({ flow }: { flow: LoginFlow }) {
  const request = useStore((s) => s.request)
  const left = useCountdown(flow.expires_at)
  return (
    <div data-testid="ai-device-flow" className="flex flex-col gap-1 rounded border p-2 text-xs">
      <span>Open the page below and enter this code:</span>
      <code data-testid="ai-user-code" className="font-mono text-base tracking-widest">
        {flow.user_code}
      </code>
      {flow.verification_uri !== null && (
        <a
          href={flow.verification_uri}
          target="_blank"
          rel="noreferrer"
          data-testid="ai-verification-uri"
          className="underline"
        >
          {flow.verification_uri}
        </a>
      )}
      {left !== undefined && <span data-testid="ai-countdown">Expires in {left}s</span>}
      <Button
        size="sm"
        variant="outline"
        data-testid="ai-login-cancel"
        onClick={() => void request({ type: 'AiProviderLoginCancel' })}
      >
        Cancel
      </Button>
    </div>
  )
}

function KeyEntry({ provider }: { provider: Provider }) {
  const request = useStore((s) => s.request)
  const [key, setKey] = useState('')
  return (
    <form
      className="flex gap-2"
      onSubmit={(e) => {
        e.preventDefault()
        if (key.trim() === '') return
        void request({ type: 'AiProviderSubmitApiKey', data: { provider, key } })
        // The secret never lingers in UI state once handed to the backend.
        setKey('')
      }}
    >
      <Input
        type="password"
        autoComplete="off"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="API key"
        aria-label={`${PROVIDER_LABEL[provider]} API key`}
        data-testid={`ai-key-input-${provider}`}
      />
      <Button type="submit" size="sm" data-testid={`ai-key-submit-${provider}`}>
        Verify
      </Button>
    </form>
  )
}

export function ProviderRow({ status }: { status: ProviderStatus }) {
  const request = useStore((s) => s.request)
  const flow = useStore((s) => s.ai.loginFlow)
  const active = useStore((s) => s.ai.active)
  const p = status.provider
  const connected = status.state.type === 'Connected'
  const activeFlow = flow?.provider === p && ['Pending', 'Polling'].includes(flow.phase.type)
  const failed =
    flow?.provider === p && ['Denied', 'Expired', 'Failed'].includes(flow.phase.type)
      ? flow.phase
      : undefined

  return (
    <li data-testid={`ai-provider-${p}`} className="flex flex-col gap-2 border-b py-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{PROVIDER_LABEL[p]}</span>
        <span data-testid={`ai-state-${p}`} className="text-xs text-muted-foreground">
          {stateLabel(status.state)}
          {status.account_label !== null && connected ? ` — ${status.account_label}` : ''}
        </span>
      </div>
      {failed !== undefined && (
        <p data-testid={`ai-flow-${failed.type}`} className="text-xs text-destructive">
          {failed.type === 'Failed' ? failed.data.message : `Sign-in ${failed.type.toLowerCase()}`}
        </p>
      )}
      {activeFlow && flow.kind === 'device_code' && <DeviceCode flow={flow} />}
      {!connected && !activeFlow && p === 'github' && (
        <Button
          size="sm"
          data-testid="ai-login-github"
          onClick={() => void request({ type: 'AiProviderLoginStart', data: { provider: p } })}
        >
          Sign in with GitHub
        </Button>
      )}
      {!connected && (p === 'claude' || p === 'openai') && <KeyEntry provider={p} />}
      {connected && (
        <div className="flex gap-2">
          {active !== p && (
            <Button
              size="sm"
              variant="outline"
              data-testid={`ai-use-${p}`}
              onClick={() => void request({ type: 'AiSetActiveProvider', data: { provider: p } })}
            >
              Use this
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            data-testid={`ai-signout-${p}`}
            onClick={() => void request({ type: 'AiProviderSignOut', data: { provider: p } })}
          >
            Sign out
          </Button>
        </div>
      )}
    </li>
  )
}
