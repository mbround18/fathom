/**
 * Inventories and their revisions (spec 055 US4, T059).
 *
 * The list follows `Inventories`; the open inventory follows `InventoryRevision` (the revision
 * shown and the history). Edits are checked as they are typed: `validate` sends
 * `ValidateInventory` after a short pause, and only the answer to the newest check counts, so a
 * slow reply never shows problems for text that has since changed. Saving, restoring and
 * deprecating are two-step, as everywhere else: a `Preview*` command answers a `Preview`, shown in
 * a confirm dialog, then `Confirm`. Inventory changes never ask for step-up.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import type {
  Cmd,
  InventoryChange,
  InventoryHostInfo,
  InventoryInfo,
  InventoryProblem,
  InventoryRevisionInfo,
  InventoryRevisionSummary,
  Preview,
  RequestId,
} from '@/protocol/wire'

const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'
/** How long typing pauses before the text is checked. */
const VALIDATE_AFTER_MS = 300

export interface Opened {
  inventory: InventoryInfo
  revision: InventoryRevisionInfo
  history: InventoryRevisionSummary[]
}

/** The newest check's answer, for the text it checked. */
export interface Validation {
  text: string
  hosts: InventoryHostInfo[]
  errors: InventoryProblem[]
  warnings: InventoryProblem[]
}

export interface Diff {
  inventory: string
  from: number
  to: number
  changes: InventoryChange[]
}

export interface InventoryState {
  workspace: string | null
  items: InventoryInfo[]
  opened: Opened | null
  /** The text waiting for, or holding, the newest check. */
  checking: string | null
  validation: Validation | null
  diff: Diff | null
  preview: Preview | null
  error: string | null
  load: (workspace: string) => void
  /** Opens an inventory at `seq`, or its newest revision. `null` closes it. */
  open: (inventory: string | null, seq?: number | null) => void
  validate: (text: string) => void
  compare: (from: number, to: number) => void
  propose: (cmd: Cmd) => void
  confirm: () => void
  cancel: () => void
  clearError: () => void
}

const mine = new Set<RequestId>()
let latestCheck: { id: RequestId; text: string } | null = null
let timer: ReturnType<typeof setTimeout> | null = null

function send(cmd: Cmd): RequestId | null {
  const id = sendCmd(cmd)
  if (id === null) {
    useInventory.setState({ error: OFFLINE })
    return null
  }
  mine.add(id)
  return id
}

export const useInventory = create<InventoryState>()((set, get) => ({
  workspace: null,
  items: [],
  opened: null,
  checking: null,
  validation: null,
  diff: null,
  preview: null,
  error: null,

  load: (workspace) => {
    if (get().workspace !== workspace) {
      set({ workspace, items: [], opened: null, diff: null, validation: null, checking: null })
    }
    send({ type: 'ListInventories', data: { workspace } })
  },
  open: (inventory, seq = null) => {
    if (inventory === null) {
      set({ opened: null, diff: null, validation: null, checking: null })
      return
    }
    if (get().opened?.inventory.id !== inventory) set({ diff: null, validation: null, checking: null })
    send({ type: 'GetInventoryRevision', data: { inventory, seq } })
  },
  validate: (text) => {
    const { workspace, opened } = get()
    if (timer !== null) clearTimeout(timer)
    set({ checking: text })
    if (!workspace) return
    timer = setTimeout(() => {
      timer = null
      const id = send({
        type: 'ValidateInventory',
        data: { workspace, content: text, inventory: opened?.inventory.id ?? null },
      })
      if (id !== null) latestCheck = { id, text }
    }, VALIDATE_AFTER_MS)
  },
  compare: (from, to) => {
    const inventory = get().opened?.inventory.id
    if (!inventory) return
    set({ error: null })
    send({ type: 'DiffInventory', data: { inventory, from, to } })
  },
  propose: (cmd) => {
    set({ error: null, preview: null })
    send(cmd)
  },
  confirm: () => {
    const p = get().preview
    if (p) send({ type: 'Confirm', data: { preview_id: p.preview_id } })
  },
  cancel: () => {
    set({ preview: null })
  },
  clearError: () => {
    set({ error: null })
  },
}))

function upsert(items: InventoryInfo[], item: InventoryInfo): InventoryInfo[] {
  return items.some((x) => x.id === item.id) ? items.map((x) => (x.id === item.id ? item : x)) : [...items, item]
}

onServerEvent(({ reply_to, event }) => {
  const s = useInventory.getState()
  const ours = reply_to !== undefined && mine.has(reply_to)
  switch (event.type) {
    case 'Inventories': {
      const items = event.data.items.filter((i) => s.workspace === null || i.workspace === s.workspace)
      const open = s.opened && items.find((i) => i.id === s.opened?.inventory.id)
      useInventory.setState({ items, ...(s.opened && open ? { opened: { ...s.opened, inventory: open } } : {}) })
      break
    }
    case 'InventoryRevision': {
      const { inventory } = event.data
      if (s.workspace !== null && inventory.workspace !== s.workspace) break
      const showing = ours || s.opened?.inventory.id === inventory.id
      useInventory.setState({
        items: upsert(s.items, inventory),
        ...(showing ? { opened: event.data } : {}),
      })
      break
    }
    case 'InventoryDiff':
      if (ours) useInventory.setState({ diff: event.data })
      break
    case 'InventoryValidation':
      if (latestCheck !== null && reply_to === latestCheck.id) {
        useInventory.setState({ validation: { text: latestCheck.text, ...event.data } })
      }
      break
    case 'Preview':
      if (ours) useInventory.setState({ preview: event.data, error: null })
      break
    case 'Confirmed':
      if (ours) useInventory.setState({ preview: null })
      break
    case 'OpError':
      if (ours) useInventory.setState({ error: event.data.message, preview: null })
      break
    case 'Denied':
      if (ours) useInventory.setState({ error: `Not allowed: ${event.data.reason}`, preview: null })
      break
    default:
      break
  }
})

onReconnected(() => {
  const { workspace, opened } = useInventory.getState()
  if (workspace) useInventory.getState().load(workspace)
  if (opened) useInventory.getState().open(opened.inventory.id, opened.revision.summary.seq)
})
