/**
 * The Repository page's address (spec 066 T051): the tab in `?tab=` and the selected item in
 * `?item=`, next to `?ws=`, so a selection is a link.
 */
import { navigate, useSearchParam } from '@companion/ui/kit'

export const TABS = ['playbooks', 'roles', 'inventory', 'secrets', 'jobs'] as const
export type RepoTab = (typeof TABS)[number]

export function isTab(value: string | null): value is RepoTab {
  return TABS.some((t) => t === value)
}

/** `/repository` with these parameters changed (`null` removes one), the rest of the query kept. */
export function repositoryHref(params: Partial<Record<'tab' | 'item', string | null>>): string {
  const url = new URL(globalThis.location.href)
  url.pathname = '/repository'
  for (const [k, v] of Object.entries(params)) {
    if (v === null) url.searchParams.delete(k)
    else url.searchParams.set(k, v)
  }
  return `${url.pathname}${url.search}`
}

/** Selects `item` (or none) on the current tab, replacing the history entry. */
export function select(item: string | null): void {
  navigate(repositoryHref({ item }), { replace: true })
}

/** Opens `item` on `tab`: a link between tabs, so it is a new history entry. */
export function goTo(tab: RepoTab, item: string | null): void {
  navigate(repositoryHref({ tab, item }))
}

export function useSelected(): string | null {
  return useSearchParam('item')
}
