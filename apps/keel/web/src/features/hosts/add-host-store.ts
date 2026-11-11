/**
 * Add host (spec 066 T053): whether the dialog is open, the group it was opened from ("Add host
 * here" in an Inventory group's detail), and what Confirm answered (`HostAdded`: the enrolment and
 * what became of the inventory change). Opened from Hosts and from the Repository page.
 */
import { create } from 'zustand'

import type { ChangeOutcome, EnrollmentIssued } from '@/protocol/wire'

export interface HostAddedResult {
  enrollment: EnrollmentIssued
  change: ChangeOutcome
}

/** The form, kept here so it survives the dialog stepping aside for the review. */
export interface AddHostDraft {
  name: string
  groups: string[]
  profiles: string[]
}

const EMPTY: AddHostDraft = { name: '', groups: [], profiles: ['base'] }

interface AddHostState {
  open: boolean
  draft: AddHostDraft
  result: HostAddedResult | null
  /** Opens the form, with `group` chosen when opened from one. */
  show: (group?: string | null) => void
  edit: (change: Partial<AddHostDraft>) => void
  close: () => void
  added: (result: HostAddedResult) => void
}

export const useAddHost = create<AddHostState>()((set) => ({
  open: false,
  draft: EMPTY,
  result: null,
  show: (group = null) => {
    set({ open: true, draft: { ...EMPTY, groups: group === null ? [] : [group] }, result: null })
  },
  edit: (change) => {
    set((s) => ({ draft: { ...s.draft, ...change } }))
  },
  close: () => {
    set({ open: false, draft: EMPTY, result: null })
  },
  added: (result) => {
    set({ result })
  },
}))
