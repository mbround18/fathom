import type { BlockedReason, Provider, ProviderState } from '@/protocol/types'

export const PROVIDER_LABEL: Record<Provider, string> = {
  github: 'GitHub Models',
  claude: 'Claude (Anthropic)',
  openai: 'OpenAI',
  ollama: 'Ollama (local)',
}

const BLOCKED: Record<BlockedReason, string> = {
  offline: 'Blocked by offline mode',
  store_locked: 'Credential store is locked — unlock your keyring',
  store_unavailable: 'No credential store available',
}

export function stateLabel(state: ProviderState): string {
  switch (state.type) {
    case 'NotConfigured':
      return 'Not signed in'
    case 'Connecting':
      return 'Connecting…'
    case 'Connected':
      return 'Connected'
    case 'NeedsReauth':
      return 'Sign in again'
    case 'Blocked':
      return BLOCKED[state.data.reason]
  }
}

export const KEY_PROVIDERS: readonly Provider[] = ['claude', 'openai']
