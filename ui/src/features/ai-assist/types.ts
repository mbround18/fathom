import type {
  CredentialMigrationOutcome,
  LoginFlow,
  OllamaState,
  Provider,
  ProviderStatus,
} from '@/protocol/types'

export interface AiErrorInfo {
  code: string
  message: string
  hint: string
}

/** Store slice for 038 provider login and Ollama detection. Holds no secrets. */
export interface AiSlice {
  loaded: boolean
  statuses: ProviderStatus[]
  active: Provider | null
  offline: boolean
  ollamaModel: string | null
  loginFlow?: LoginFlow
  ollama?: OllamaState
  migration?: CredentialMigrationOutcome
  lastError?: AiErrorInfo
}
