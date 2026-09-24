/**
 * 016-command-palette: Cmd/Ctrl+K fuzzy jump to a resource, namespace, or nav section.
 *
 * Pure presentation + selection wiring; all candidate-building/ranking logic lives in
 * `@/resources/paletteIndex` (unit-tested there per constitution III).
 */
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
  parsePaletteQuery,
  rankAndFilter,
  type PaletteNamespacePayload,
  type PaletteResourcePayload,
  type PaletteResult,
  type PaletteSectionPayload,
} from '@/resources/paletteIndex'
import { useStore } from '@/store'

export interface CommandPaletteProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

const CATEGORY_HEADING: Record<PaletteResult['category'], string> = {
  resource: 'Resources',
  namespace: 'Namespaces',
  section: 'Sections',
}

/** Group results into the three fixed sections, in a stable display order. */
function groupByCategory(results: readonly PaletteResult[]) {
  const groups: Record<PaletteResult['category'], PaletteResult[]> = {
    resource: [],
    namespace: [],
    section: [],
  }
  for (const result of results) groups[result.category].push(result)
  return (['resource', 'namespace', 'section'] as const)
    .map((category) => ({ category, results: groups[category] }))
    .filter((g) => g.results.length > 0)
}

export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const lists = useStore((s) => s.lists)
  const activeContext = useStore((s) => s.session.activeContext)
  const namespaces = useStore((s) => s.session.namespaces)
  const openInspector = useStore((s) => s.openInspector)
  const selectNamespace = useStore((s) => s.selectNamespace)
  const navigate = useStore((s) => s.navigate)
  const slotPanes = useStore((s) => s.layout.slotPanes)
  const panes = useStore((s) => s.panes)

  const candidates = useMemo(
    () => [
      ...buildResourceCandidates(lists, activeContext),
      ...buildNamespaceCandidates(namespaces),
      ...buildSectionCandidates(),
    ],
    [lists, activeContext, namespaces],
  )

  const results = useMemo(() => {
    const parsed = parsePaletteQuery(query)
    return rankAndFilter(candidates, parsed)
  }, [candidates, query])

  const groups = groupByCategory(results)

  function reset() {
    setQuery('')
  }

  function handleSelect(result: PaletteResult) {
    if (result.category === 'resource') {
      // Pass a visible slot list pane as `from`: on laptop/tablet the store only retargets the
      // fixed inspector slot when the jump originates from a pane in the visible row.
      const from = slotPanes.find((id) => panes[id]?.kind === 'list')
      openInspector((result.payload as PaletteResourcePayload).ref, from)
    } else if (result.category === 'namespace') {
      selectNamespace((result.payload as PaletteNamespacePayload).namespace)
    } else {
      navigate((result.payload as PaletteSectionPayload).section)
    }
    onOpenChange(false)
    reset()
  }

  return (
    <CommandDialog
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
                  <span className="truncate">{result.label}</span>
                  {result.sublabel && (
                    <span className="ml-auto truncate text-muted-foreground">
                      {result.sublabel}
                    </span>
                  )}
                </CommandItem>
              ))}
            </CommandGroup>
          ))}
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
