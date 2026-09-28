/**
 * 016-command-palette: Cmd/Ctrl+K fuzzy jump to a resource, namespace, or nav section.
 *
 * Pure presentation + selection wiring; all candidate-building/ranking logic lives in
 * `@/resources/paletteIndex` (unit-tested there per constitution III).
 */
import { useFocusReturn } from '@/lib/use-focus-return'
import { useMemo, useState } from 'react'

import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  buildNamespaceCandidates,
  buildResourceCandidates,
  buildSectionCandidates,
  buildToolCandidates,
  parsePaletteQuery,
  rankAndFilter,
  type PaletteNamespacePayload,
  type PaletteResourcePayload,
  type PaletteResult,
  type PaletteSectionPayload,
  type PaletteToolPayload,
} from '@/resources/paletteIndex'
import { useNavVisibility } from '@/resources/useNavVisibility'
import { useStore } from '@/store'

import { requestRoleCreate } from './rbac/roleCreateRequest'
import type { RoleKind } from './rbac/roleYaml'
import { useOpenTool } from './shell/useOpenTool'

/** 043 US8: palette actions that open 042's role form (through the Roles list's launcher). */
const ROLE_ACTIONS: readonly { id: string; label: string; kind: RoleKind; terms: string }[] = [
  {
    id: 'action:new-role',
    label: 'New Role…',
    kind: 'Role',
    terms: 'new role create namespaced rbac access permissions',
  },
  {
    id: 'action:new-clusterrole',
    label: 'New ClusterRole…',
    kind: 'ClusterRole',
    terms: 'new clusterrole cluster role create rbac access permissions',
  },
]

function matchingRoleActions(query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return []
  return ROLE_ACTIONS.filter((a) => words.every((w) => a.terms.includes(w)))
}

export interface CommandPaletteProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const CATEGORY_HEADING: Record<PaletteResult['category'], string> = {
  resource: 'Resources',
  namespace: 'Namespaces',
  section: 'Sections',
  tool: 'Tools',
}

/** Group results into the three fixed sections, in a stable display order. */
function groupByCategory(results: readonly PaletteResult[]) {
  const groups: Record<PaletteResult['category'], PaletteResult[]> = {
    resource: [],
    namespace: [],
    section: [],
    tool: [],
  }
  for (const result of results) groups[result.category].push(result)
  return (['resource', 'namespace', 'section', 'tool'] as const)
    .map((category) => ({ category, results: groups[category] }))
    .filter((g) => g.results.length > 0)
}

export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const returnFocus = useFocusReturn(open)
  const [query, setQuery] = useState('')
  const lists = useStore((s) => s.lists)
  const activeContext = useStore((s) => s.session.activeContext)
  const namespaces = useStore((s) => s.session.namespaces)
  const openInspector = useStore((s) => s.openInspector)
  const selectNamespace = useStore((s) => s.selectNamespace)
  const navigate = useStore((s) => s.navigate)
  const openTool = useOpenTool()
  const visibility = useNavVisibility()

  // The index (every loaded row of the active context) is only built while the palette is open:
  // building it on each list update / keystroke cost tens of ms with 10k rows for nothing.
  const candidates = useMemo(
    () =>
      !open
        ? []
        : [
            ...buildResourceCandidates(lists, activeContext),
            ...buildNamespaceCandidates(namespaces),
            ...buildSectionCandidates(visibility),
            ...buildToolCandidates(visibility),
          ],
    [open, lists, activeContext, namespaces, visibility],
  )

  const results = useMemo(() => {
    const parsed = parsePaletteQuery(query)
    return rankAndFilter(candidates, parsed)
  }, [candidates, query])

  const groups = groupByCategory(results)
  const actions = matchingRoleActions(query)

  function reset() {
    setQuery('')
  }

  function handleSelect(result: PaletteResult) {
    if (result.category === 'resource') {
      // Pass a visible slot list pane as `from`: on laptop/tablet the store only retargets the
      // fixed inspector slot when the jump originates from a pane in the visible row.
      const { layout, panes } = useStore.getState()
      const from = layout.slotPanes.find((id) => panes[id]?.kind === 'list')
      openInspector((result.payload as PaletteResourcePayload).ref, from)
    } else if (result.category === 'namespace') {
      selectNamespace((result.payload as PaletteNamespacePayload).namespace)
    } else if (result.category === 'tool') {
      openTool((result.payload as PaletteToolPayload).tool)
    } else {
      navigate((result.payload as PaletteSectionPayload).section)
    }
    onOpenChange(false)
    reset()
  }

  return (
    <CommandDialog
      className="sm:max-w-[600px]"
      onCloseAutoFocus={returnFocus}
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next)
        if (!next) reset()
      }}
    >
      <Command shouldFilter={false} loop>
        <CommandInput
          placeholder="Jump to a resource, namespace, or section… (try kind:name)"
          value={query}
          onValueChange={setQuery}
        />
        <CommandList>
          <CommandEmpty>No matches.</CommandEmpty>
          {groups.map(({ category, results: groupResults }) => (
            <CommandGroup key={category} heading={CATEGORY_HEADING[category]}>
              {groupResults.map((result) => (
                <CommandItem
                  key={result.id}
                  value={result.id}
                  onSelect={() => handleSelect(result)}
                >
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate">{result.label}</span>
                    {result.description && (
                      <span
                        data-testid="palette-description"
                        className="truncate text-xs text-muted-foreground"
                      >
                        {result.description}
                      </span>
                    )}
                  </span>
                  {result.sublabel && (
                    <span className="ml-auto shrink-0 truncate text-muted-foreground">
                      {result.sublabel}
                    </span>
                  )}
                </CommandItem>
              ))}
            </CommandGroup>
          ))}
          {actions.length > 0 && (
            <CommandGroup heading="Actions">
              {actions.map((a) => (
                <CommandItem
                  key={a.id}
                  value={a.id}
                  onSelect={() => {
                    navigate(a.kind === 'Role' ? 'access/roles' : 'access/clusterroles')
                    requestRoleCreate(a.kind)
                    onOpenChange(false)
                    reset()
                  }}
                >
                  <span className="truncate">{a.label}</span>
                  <span className="ml-auto truncate text-muted-foreground">Access Control</span>
                </CommandItem>
              ))}
            </CommandGroup>
          )}
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
