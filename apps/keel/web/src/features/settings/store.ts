/**
 * System Settings state (spec 060 US2, US3, US7): the instance settings (enrolment template,
 * instructions, the read-only role mapping and its warnings), every enrolment token, every API
 * token, and the System workspace.
 *
 * Reads (`GetSettings`, `ListEnrollments`, `ListAllApiTokens`, `GetSystem`) are answered by one
 * event each, or a refusal kept beside the value and shown as sent. Changes go through the
 * governance preview → confirm dialog (`useGovernance.propose`), whose `onResult` hands back the
 * event a change answers with (`Settings`, `System`); after a `Confirmed`, the list it touched is
 * read again.
 *
 * Saves carry the version they were edited from. When the server says someone saved in between
 * (`settings_conflict`, `system_conflict`) the tab shows a banner with Reload, and nothing is
 * overwritten.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useGovernance, type Refusal } from '@/features/teams/store'
import type {
  AdminApiTokenInfo,
  ApiTokenState,
  Cmd,
  EnrollmentInfo,
  EnrollmentState,
  Event,
  RequestId,
} from '@/protocol/wire'

export type SettingsInfo = Extract<Event, { type: 'Settings' }>['data']
export type SystemInfo = Extract<Event, { type: 'System' }>['data']

/** The OpError codes that mean "someone saved first" (contracts/protocol.md). */
export const SETTINGS_CONFLICT = 'settings_conflict'
export const SYSTEM_CONFLICT = 'system_conflict'

/** One thing read from the server: `null` until it arrives. */
export interface Loaded<T> {
  value: T | null
  refusal: Refusal | null
}

const EMPTY = { value: null, refusal: null }

export interface SettingsState {
  settings: Loaded<SettingsInfo>
  enrollments: Loaded<EnrollmentInfo[]>
  apiTokens: Loaded<AdminApiTokenInfo[]>
  system: Loaded<SystemInfo>
  enrollmentFilter: EnrollmentState | null
  apiTokenFilter: ApiTokenState | null
  /** Someone saved the settings after they were loaded here. */
  settingsConflict: boolean
  systemConflict: boolean

  loadSettings: () => void
  loadEnrollments: (filter?: EnrollmentState | null) => void
  loadApiTokens: (filter?: ApiTokenState | null) => void
  loadSystem: () => void

  saveSettings: (template: string, instructions: string) => void
  revokeEnrollment: (enrollment: EnrollmentInfo) => void
  revokeApiToken: (token: AdminApiTokenInfo) => void
  configureSystem: (allowed_profiles: string[]) => void
  removeSystem: () => void
}

const OFFLINE: Refusal = {
  kind: 'error',
  code: 'offline',
  message: 'Keel is not connected. Wait for it to reconnect, then try again.',
}

type Key = 'settings' | 'enrollments' | 'apiTokens' | 'system'

const handlers = new Map<RequestId, (event: Event) => void>()
/** What has been asked for, so a reconnect asks again. */
const asked = new Set<Key>()

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) handlers.get(reply_to)?.(event)
  // A conflict can answer the preview or the confirm; either way the tab shows the banner.
  if (event.type === 'OpError') {
    if (event.data.code === SETTINGS_CONFLICT) useSettings.setState({ settingsConflict: true })
    if (event.data.code === SYSTEM_CONFLICT) useSettings.setState({ systemConflict: true })
  }
})

onReconnected(() => {
  const s = useSettings.getState()
  if (asked.has('settings')) s.loadSettings()
  if (asked.has('enrollments')) s.loadEnrollments()
  if (asked.has('apiTokens')) s.loadApiTokens()
  if (asked.has('system')) s.loadSystem()
})

function refusalOf(event: Event): Refusal | null {
  if (event.type === 'Denied') return { kind: 'denied', action: event.data.action, reason: event.data.reason }
  if (event.type === 'OpError') return { kind: 'error', code: event.data.code, message: event.data.message }
  return null
}

/** Sends a read; its answer (or refusal) lands in `key`. */
function read<K extends Key>(key: K, cmd: Cmd, pick: (event: Event) => SettingsState[K]['value'] | undefined): void {
  asked.add(key)
  const id = sendCmd(cmd)
  if (id === null) {
    useSettings.setState({ [key]: { value: useSettings.getState()[key].value, refusal: OFFLINE } })
    return
  }
  handlers.set(id, (event) => {
    const value = pick(event)
    if (value !== undefined) {
      handlers.delete(id)
      useSettings.setState({ [key]: { value, refusal: null } })
      return
    }
    const refusal = refusalOf(event)
    if (refusal) {
      handlers.delete(id)
      useSettings.setState({ [key]: { value: null, refusal } })
    }
  })
}

/** The System workspace came or went: re-read it, and the workspace list that shows it. */
function systemChanged(): void {
  useSettings.getState().loadSystem()
  useGovernance.getState().load('workspaces')
}

export const useSettings = create<SettingsState>()((set, get) => ({
  settings: EMPTY,
  enrollments: EMPTY,
  apiTokens: EMPTY,
  system: EMPTY,
  enrollmentFilter: null,
  apiTokenFilter: null,
  settingsConflict: false,
  systemConflict: false,

  loadSettings: () => {
    set({ settingsConflict: false })
    read('settings', { type: 'GetSettings' }, (e) => (e.type === 'Settings' ? e.data : undefined))
  },

  loadEnrollments: (filter) => {
    const state = filter === undefined ? get().enrollmentFilter : filter
    set({ enrollmentFilter: state })
    read('enrollments', { type: 'ListEnrollments', data: { state, workspace: null } }, (e) =>
      e.type === 'Enrollments' ? e.data.items : undefined,
    )
  },

  loadApiTokens: (filter) => {
    const state = filter === undefined ? get().apiTokenFilter : filter
    set({ apiTokenFilter: state })
    read('apiTokens', { type: 'ListAllApiTokens', data: { holder: null, state } }, (e) =>
      e.type === 'AllApiTokens' ? e.data.items : undefined,
    )
  },

  loadSystem: () => {
    set({ systemConflict: false })
    read('system', { type: 'GetSystem' }, (e) => (e.type === 'System' ? e.data : undefined))
  },

  saveSettings: (template, instructions) => {
    const version = get().settings.value?.version ?? 0
    useGovernance.getState().propose(
      'Save enrolment settings',
      { type: 'PreviewSettingsUpdate', data: { template, instructions, version } },
      (event) => {
        if (event.type === 'Settings') set({ settings: { value: event.data, refusal: null }, settingsConflict: false })
        else if (event.type === 'Confirmed') get().loadSettings()
      },
    )
  },

  revokeEnrollment: (enrollment) => {
    useGovernance.getState().propose(
      `Revoke the token for ${enrollment.inventory_name}`,
      { type: 'PreviewEnrollmentRevoke', data: { enrollment: enrollment.id } },
      // The confirm answers the unfiltered list; read it again under the current filter instead.
      (event) => {
        if (event.type === 'Confirmed') get().loadEnrollments()
      },
    )
  },

  revokeApiToken: (token) => {
    useGovernance.getState().propose(
      `Revoke API token ${token.token.name}`,
      { type: 'PreviewApiTokenRevoke', data: { token: token.token.id } },
      (event) => {
        if (event.type === 'Confirmed') get().loadApiTokens()
      },
    )
  },

  configureSystem: (allowed_profiles) => {
    const version = get().system.value?.version ?? 0
    const configured = get().system.value?.configured === true
    useGovernance.getState().propose(
      configured ? 'Change System profiles' : 'Set up the System workspace',
      { type: 'PreviewSystemConfigure', data: { allowed_profiles, version } },
      (event) => {
        if (event.type === 'System') set({ system: { value: event.data, refusal: null }, systemConflict: false })
        else if (event.type === 'Confirmed') systemChanged()
      },
    )
  },

  removeSystem: () => {
    useGovernance.getState().propose('Remove the System workspace', { type: 'PreviewSystemRemove' }, (event) => {
      if (event.type === 'System') set({ system: { value: event.data, refusal: null } })
      else if (event.type === 'Confirmed') systemChanged()
    })
  },
}))
