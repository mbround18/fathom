/**
 * The current workspace's catalog and declared jobs (spec 066 T051). `GetCatalog` answers
 * `Catalog`, or refuses `no_primary_repository` / `no_catalog`, which the page shows as its empty
 * states; `ListJobs` on the primary binding answers `Jobs`. Both load again when the primary
 * binding syncs, so a push shows up without a reload. Hosts and Add host read the same catalog.
 */
import { create } from 'zustand'

import { onServerEvent } from '@/app/connection'
import { loadList, type Refusal } from '@/app/load'
import type { CatalogView, JobInfo, WorkspaceInfo } from '@/protocol/wire'

export interface CatalogState {
  workspace: string | null
  binding: string | null
  /** `undefined` while loading. */
  catalog: CatalogView | undefined
  failed: Refusal | null
  /** `null` while loading, or with no primary binding. */
  jobs: JobInfo[] | null
  jobsFailed: Refusal | null
  load: (workspace: Pick<WorkspaceInfo, 'id' | 'primary_binding'>) => void
}

export const useCatalog = create<CatalogState>()((set, get) => ({
  workspace: null,
  binding: null,
  catalog: undefined,
  failed: null,
  jobs: null,
  jobsFailed: null,

  load: ({ id, primary_binding }) => {
    if (get().workspace !== id || get().binding !== primary_binding) {
      set({ workspace: id, binding: primary_binding, catalog: undefined, jobs: null })
    }
    set({ failed: null, jobsFailed: null })
    const current = () => get().workspace === id
    loadList({ type: 'GetCatalog', data: { workspace: id } }, (e) => (e.type === 'Catalog' ? e.data : undefined), {
      loaded: (catalog) => {
        if (current()) set({ catalog })
      },
      failed: (failed) => {
        if (current()) set({ failed, catalog: undefined })
      },
    })
    if (primary_binding === null) return
    loadList({ type: 'ListJobs', data: { binding: primary_binding } }, (e) => (e.type === 'Jobs' ? e.data.items : undefined), {
      loaded: (jobs) => {
        if (current()) set({ jobs })
      },
      failed: (jobsFailed) => {
        if (current()) set({ jobsFailed })
      },
    })
  },
}))

/** The refusal code a failed catalog load carries, if it was the server's. */
export function refusalCode(failed: Refusal | null): string | null {
  return failed?.kind === 'error' ? failed.code : null
}

onServerEvent(({ event }) => {
  if (event.type !== 'BindingSynced') return
  const s = useCatalog.getState()
  if (s.workspace !== null && s.binding === event.data.binding) s.load({ id: s.workspace, primary_binding: s.binding })
})
