/**
 * 032-ui-shell-a11y: derive the `AgentObjectRef` the top-bar "Ask AI" sheet is seeded with from
 * what the operator is looking at: the selected object when one is open, else the visible
 * section (kind `Section`), else the bare cluster context (kind `Cluster`).
 */
import { useMemo } from 'react'

import type { AgentObjectRef } from '@/protocol/types'
import { sectionLabel } from '@/resources/nav'
import { useStore } from '@/store'

import { AGENT_OPS_KINDS } from './kinds'

export interface AskAiContext {
  ref: AgentObjectRef
  /** Short chip text, e.g. `kind-fathom / payments / Pod api-7f9`. */
  chip: string
}

export function useAskAiContext(): AskAiContext {
  const context = useStore((s) => s.session.activeContext)
  const namespace = useStore((s) => s.session.activeNamespace)
  const panes = useStore((s) => s.panes)
  const slotPanes = useStore((s) => s.layout.slotPanes)
  const stack = useStore((s) => s.layout.stack)
  const focusedList = useStore((s) => s.layout.focusedList)

  return useMemo(() => {
    const visible = [...stack, ...slotPanes].map((id) => panes[id])
    const target = visible.find((p) => p?.kind === 'inspector' && p.target)?.target
    const list =
      (focusedList ? panes[focusedList] : undefined) ?? visible.find((p) => p?.kind === 'list')
    const ctx = context ?? 'no context'
    if (target && AGENT_OPS_KINDS.has(target.kind) && target.namespace) {
      return {
        ref: { kind: target.kind, namespace: target.namespace, name: target.name },
        chip: `${ctx} / ${target.namespace} / ${target.kind} ${target.name}`,
      }
    }
    const section = list?.route?.section
    if (section) {
      return {
        ref: { kind: 'Section', namespace: namespace ?? '', name: section },
        chip: `${ctx} / ${namespace ?? 'all namespaces'} / ${sectionLabel(section)}`,
      }
    }
    return {
      ref: { kind: 'Cluster', namespace: namespace ?? '', name: ctx },
      chip: `${ctx} / ${namespace ?? 'all namespaces'}`,
    }
  }, [context, namespace, panes, slotPanes, stack, focusedList])
}
