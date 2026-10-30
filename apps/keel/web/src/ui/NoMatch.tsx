/**
 * A list a filter emptied (spec 063 FR-014): says so plainly and clears the filter, rather than
 * showing the guided "nothing here yet" state for a list that isn't empty.
 */
import { Button } from '@companion/ui'

export function NoMatch({ what, onClear }: { what: string; onClear: () => void }) {
  return (
    <div data-pane-state="empty" data-empty="no-match" className="flex min-w-0 flex-wrap items-center gap-2 text-sm text-muted-foreground">
      <span>No {what} match.</span>
      <Button type="button" variant="ghost" size="xs" onClick={onClear}>
        Clear filter
      </Button>
    </div>
  )
}
