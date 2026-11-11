/**
 * The current workspace (spec 066 R1). One choice for the whole app, resolved in this order:
 *
 * 1. `?ws=<slug>` in the address, so a link opens the workspace it was copied from;
 * 2. `localStorage["keel.workspace"]`, so a reload stays put;
 * 3. the first visible workspace.
 *
 * An archived or vanished workspace is skipped at every step. The list is `ListWorkspaces`, not
 * `me.grants`: the server answers every workspace to an instance owner or admin, so an owner with
 * no grants still has one to pick.
 *
 * `useWorkspaceSync` (mounted once, by the shell's switcher) keeps the address and the stored
 * choice in line with what was resolved. It replaces the history entry and never pushes one, so
 * Back leaves the page rather than stepping through workspaces. Pages read `useCurrentWorkspace`.
 */
import { navigate, slug as slugOf, useSearchParam } from '@companion/ui/kit'
import { useEffect, useMemo } from 'react'
import { create } from 'zustand'

import { useGovernance } from '@/features/teams/store'
import type { WorkspaceInfo } from '@/protocol/wire'

export const STORAGE_KEY = 'keel.workspace'
export const WS_PARAM = 'ws'

/** A workspace slug, as the server checks it. */
export const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/

/** A slug from a name: lower case, hyphenated, at most 63 characters. */
export function slugFromName(name: string): string {
  return slugOf(name).slice(0, 63).replace(/-+$/, '')
}

/** Storage can be missing or refuse (a private window, blocked site data): the choice then lasts the tab. */
function readStored(): string | null {
  try {
    return globalThis.localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

function writeStored(slug: string | null): void {
  try {
    if (slug === null) globalThis.localStorage.removeItem(STORAGE_KEY)
    else globalThis.localStorage.setItem(STORAGE_KEY, slug)
  } catch {
    // See readStored.
  }
}

/** The workspaces offered: not archived, in slug order, the owner-only System workspace last. */
export function visibleWorkspaces(items: readonly WorkspaceInfo[]): WorkspaceInfo[] {
  return items
    .filter((w) => !w.archived)
    .sort((a, b) => Number(a.system) - Number(b.system) || a.slug.localeCompare(b.slug))
}

/** The current workspace: `asked` (the address), then `stored`, then the first visible one. */
export function resolveWorkspace(
  items: readonly WorkspaceInfo[] | null,
  asked: string | null,
  stored: string | null,
): WorkspaceInfo | null {
  if (items === null) return null
  const visible = visibleWorkspaces(items)
  const find = (slug: string | null) => (slug === null ? undefined : visible.find((w) => w.slug === slug))
  return find(asked) ?? find(stored) ?? visible[0] ?? null
}

/** `href` (path, query and fragment) with `?ws=` set to `slug`, or removed for `null`. */
export function withWorkspace(href: string, slug: string | null): string {
  const url = new URL(href, 'http://keel.invalid')
  if (slug === null) url.searchParams.delete(WS_PARAM)
  else url.searchParams.set(WS_PARAM, slug)
  return `${url.pathname}${url.search}${url.hash}`
}

function here(): string {
  const { pathname, search, hash } = globalThis.location
  return `${pathname}${search}${hash}`
}

/** Puts `slug` in the address, replacing the history entry. */
function syncAddress(slug: string | null): void {
  const to = withWorkspace(here(), slug)
  if (to !== here()) navigate(to, { replace: true })
}

interface WorkspaceStore {
  /** The stored choice, by slug. */
  stored: string | null
  /**
   * A workspace just created, made current as soon as the list holds it. Choosing it before then
   * would resolve to the fallback, and the sync would write the fallback back over it.
   */
  pending: string | null
  /** Makes `slug` current: remembered, and written to the address. */
  choose: (slug: string) => void
  /** Makes `slug` current once `ListWorkspaces` returns it. */
  chooseWhenListed: (slug: string) => void
}

export const useWorkspaceStore = create<WorkspaceStore>()((set) => ({
  stored: readStored(),
  pending: null,
  choose: (slug) => {
    writeStored(slug)
    set({ stored: slug, pending: null })
    syncAddress(slug)
  },
  chooseWhenListed: (slug) => {
    set({ pending: slug })
  },
}))

/** The current workspace; `null` while the list loads, and when there is none. */
export function useCurrentWorkspace(): WorkspaceInfo | null {
  const items = useGovernance((s) => s.workspaces)
  const asked = useSearchParam(WS_PARAM)
  const stored = useWorkspaceStore((s) => s.stored)
  return useMemo(() => resolveWorkspace(items, asked, stored), [items, asked, stored])
}

/**
 * Loads the workspace list and keeps the address and the stored choice on the resolved workspace.
 * Mount it once; it runs again when a navigation drops `?ws=` or the list changes under it.
 */
export function useWorkspaceSync(): void {
  const load = useGovernance((s) => s.load)
  const items = useGovernance((s) => s.workspaces)
  const loaded = items !== null
  const pending = useWorkspaceStore((s) => s.pending)
  const asked = useSearchParam(WS_PARAM)
  const current = useCurrentWorkspace()
  const slug = current?.slug ?? null

  useEffect(() => {
    load('workspaces')
  }, [load])

  useEffect(() => {
    if (pending !== null && items && visibleWorkspaces(items).some((w) => w.slug === pending)) {
      useWorkspaceStore.getState().choose(pending)
    }
  }, [pending, items])

  useEffect(() => {
    if (!loaded || pending !== null) return
    if (slug !== null && useWorkspaceStore.getState().stored !== slug) {
      writeStored(slug)
      useWorkspaceStore.setState({ stored: slug })
    }
    if (asked !== slug) syncAddress(slug)
  }, [loaded, pending, asked, slug])
}
