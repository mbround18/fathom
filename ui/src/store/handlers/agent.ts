/**
 * `AgentConversationUpdated` (013-agent-sdk-ops): keyed by `${kind}/${namespace}/${name}`
 * (matches the backend's `conversation_key`), rather than the generic `details` keying (whose
 * `readKeyOf` assumes a payload shape this event doesn't carry — same problem
 * `ContainerImageInspected` has, see `handlers/imageInspect.ts`).
 */
import type { ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'

export function agentConversationKey(kind: string, namespace: string, name: string): string {
  return `${kind}/${namespace}/${name}`
}

type AgentUpdated = Extract<ResourceReadEvent, { type: 'AgentConversationUpdated' }>

export function handleAgentConversation(d: Draft, data: AgentUpdated['data']): void {
  const { object, conversation } = data
  const key = agentConversationKey(object.kind, object.namespace, object.name)
  d.edit('agentConversations')[key] = conversation
}
