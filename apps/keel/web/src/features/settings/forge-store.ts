/**
 * The current workspace's forge connection (spec 066 T034, contracts/ui.md "Settings → Workspace").
 * `GetForgeConnection` answers `ForgeConnection` (the connection, or `null` for none); `ProbeForge`
 * answers `ForgeProbe`, shown under Test until the next load. Changes go through the governance
 * `ChangeDialog`; the tab loads again once one is confirmed.
 */
import { create } from 'zustand'

import { loadList, type Refusal } from '@/app/load'
import type { ForgeConnectionInfo, ForgeProvider } from '@/protocol/wire'

/**
 * The API base the server falls back to when none is given: GitHub's public API, or the primary
 * repository's origin with `/api/v1` for Forgejo and Gitea. `null` when there is nothing to derive
 * it from (an SSH URL with no host scheme, or no repository yet).
 */
export function defaultApiBase(provider: ForgeProvider, repoUrl: string | null): string | null {
  if (provider === 'github') return 'https://api.github.com'
  if (repoUrl === null) return null
  try {
    const url = new URL(repoUrl)
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return null
    return `${url.origin}/api/v1`
  } catch {
    return null
  }
}

export interface ProbeResult {
  ok: boolean
  can_push: boolean
  message: string | null
}

export interface ForgeState {
  workspace: string | null
  /** `undefined` while loading, `null` when the workspace has none. */
  connection: ForgeConnectionInfo | null | undefined
  failed: Refusal | null
  probing: boolean
  probe: ProbeResult | Refusal | null
  load: (workspace: string) => void
  test: () => void
}

export const useForge = create<ForgeState>()((set, get) => ({
  workspace: null,
  connection: undefined,
  failed: null,
  probing: false,
  probe: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, connection: undefined, probe: null, probing: false })
    set({ failed: null })
    loadList(
      { type: 'GetForgeConnection', data: { workspace } },
      (event) => (event.type === 'ForgeConnection' ? { connection: event.data } : undefined),
      {
        loaded: ({ connection }) => {
          if (get().workspace === workspace) set({ connection, failed: null })
        },
        failed: (failed) => {
          if (get().workspace === workspace) set({ failed })
        },
      },
    )
  },

  test: () => {
    const { workspace } = get()
    if (workspace === null) return
    set({ probing: true, probe: null })
    loadList(
      { type: 'ProbeForge', data: { workspace } },
      (event) => (event.type === 'ForgeProbe' ? event.data : undefined),
      {
        loaded: (probe) => {
          if (get().workspace === workspace) set({ probing: false, probe })
        },
        failed: (probe) => {
          if (get().workspace === workspace) set({ probing: false, probe })
        },
      },
    )
  },
}))
