/**
 * The one-time "this leaves your machine" disclosure before the first Ask AI request to a
 * hosted provider (Anthropic / OpenAI-compatible / GitHub).
 *
 * What the backend sends with every `AgentAsk` (`src/backend/agent/grounding.rs`): the
 * question, the resource's status, its recent pod logs and the Kubernetes events involving
 * it. For a hosted provider that is a third-party upload, so the operator is told once and
 * asked to confirm; local Ollama never prompts. Acceptance lives in `UiConfig`
 * (`aiDisclosureAccepted`), the same persisted preference store as theme and layout.
 */
import type { AgentAuthMode, Provider } from '@/protocol/types'

export const HOSTED_PROVIDERS: ReadonlySet<Provider> = new Set<Provider>([
  'github',
  'claude',
  'openai',
])

export interface DisclosureInputs {
  /** 038: the provider the backend will dispatch to (`AiProviderStatus.active`). */
  active: Provider | null
  /** 013: the legacy auth mode, consulted only when 038 has no active provider. */
  legacyMode: AgentAuthMode | undefined
  /** `UiConfig.aiDisclosureAccepted`. */
  accepted: boolean | undefined
}

/** True when the request would go to a hosted provider and the operator has not yet agreed. */
export function needsHostedDisclosure({ active, legacyMode, accepted }: DisclosureInputs): boolean {
  if (accepted) return false
  if (active !== null) return HOSTED_PROVIDERS.has(active)
  return legacyMode?.type === 'Anthropic'
}

export const DISCLOSURE_TITLE = 'Send this resource to the AI provider?'

export const DISCLOSURE_TEXT =
  'Your question, this resource’s status, its recent pod logs and the Kubernetes events ' +
  'involving it are sent to the hosted AI provider you signed in to. Obvious secrets ' +
  '(tokens, keys, passwords, private keys) are scrubbed first, but scrubbing is best-effort: ' +
  'anything else in those logs goes as-is. You are asked once; local Ollama never sends ' +
  'anything off this machine.'
