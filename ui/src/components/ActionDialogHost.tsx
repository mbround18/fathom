/**
 * Renders the pending `store.actionPrompt` as a confirm or scale dialog (T092/T095) and, once the
 * operator confirms, hands it to `runAction`. One host for the whole app, mounted by the shell.
 * Focus goes back where the action came from (the row's grid or the menu trigger) on close.
 */
import { descriptorFor, type AnyDescriptor } from '@/resources'
import type { ActionDescriptor } from '@/resources'
import { runAction, summarize } from '@/store/actions'
import { useStore, type ActionPrompt, type ListRow } from '@/store'

import { ConfirmDialog } from './ConfirmDialog'
import { ScaleDialog } from './ScaleDialog'

function restoreFocus(prompt: ActionPrompt): void {
  const target = prompt.returnFocus
  if (target?.isConnected) {
    target.focus()
    return
  }
  if (prompt.paneId !== undefined) {
    document.querySelector<HTMLElement>(`[data-pane-id="${prompt.paneId}"] [role="grid"]`)?.focus()
  }
}

function detailFor(
  descriptor: AnyDescriptor,
  action: ActionDescriptor<ListRow>,
  rows: ListRow[],
): string | undefined {
  if (action.id === 'restart') {
    return 'Starts a rolling restart: the controller replaces the pods one by one.'
  }
  if (action.id === 'delete' && rows.length > 1) {
    const plural = descriptor.label.toLowerCase()
    return action.plan(rows, {}).length === 1
      ? `Sent as one bulk delete for all ${String(rows.length)} ${plural}.`
      : `Sent as ${String(rows.length)} separate deletes, one per object.`
  }
  return undefined
}

export function ActionDialogHost() {
  const prompt = useStore((s) => s.actionPrompt)
  const closeAction = useStore((s) => s.closeAction)
  const descriptor = descriptorFor(prompt?.section)
  const action = descriptor?.actions.find((a) => a.id === prompt?.actionId)
  if (!prompt || !descriptor || !action || prompt.rows.length === 0) return null

  const rows = [...prompt.rows]
  const items = rows.map((r) => {
    const ref = descriptor.ref(r)
    return { namespace: ref.namespace ?? null, name: ref.name }
  })
  const onCloseAutoFocus = (e: Event) => {
    e.preventDefault()
    restoreFocus(prompt)
  }
  const run = (replicas?: number) => {
    closeAction()
    void runAction(descriptor, action, rows, {
      ...(prompt.paneId !== undefined ? { paneId: prompt.paneId } : {}),
      input: replicas === undefined ? {} : { replicas },
    })
  }

  const [only] = rows
  const [item] = items
  if (action.prompt === 'scale' && only !== undefined && item !== undefined) {
    return (
      <ScaleDialog
        key={items.map((i) => i.name).join(',')}
        open
        kind={descriptor.kind}
        item={item}
        current={action.current?.(only) ?? 0}
        onConfirm={(n) => run(n)}
        onCancel={closeAction}
        onCloseAutoFocus={onCloseAutoFocus}
      />
    )
  }

  const target = summarize(descriptor, rows)
  const title =
    rows.length === 1
      ? `${action.label} ${descriptor.kind.toLowerCase()}?`
      : `${action.label} ${target}?`
  return (
    <ConfirmDialog
      key={`${action.id}:${items.map((i) => i.name).join(',')}`}
      open
      title={title}
      items={items}
      detail={detailFor(descriptor, action, rows)}
      destructive={action.destructive ?? false}
      confirmLabel={
        rows.length === 1
          ? `${action.label} ${descriptor.kind.toLowerCase()}`
          : `${action.label} ${target}`
      }
      onConfirm={() => run()}
      onCancel={closeAction}
      onCloseAutoFocus={onCloseAutoFocus}
    />
  )
}
