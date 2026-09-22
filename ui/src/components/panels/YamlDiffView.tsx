/**
 * YAML diff view (US1, T024): `liveYaml` vs `editedYaml` for a `ResourceEditSession`, using
 * `diffLines()` from the `diff` package. Live: recomputes whenever `liveYaml` changes (a
 * watch-driven update while the tab is open), and shows a "content changed" banner when that
 * happens after the operator already reviewed (`liveYamlChangedSinceReview`).
 */
import { diffLines } from 'diff'
import { useMemo } from 'react'

import { cn } from '@/lib/utils'
import type { ResourceEditSession } from '@/store'

export function YamlDiffView({ session }: { session: ResourceEditSession }) {
  const parts = useMemo(
    () => diffLines(session.liveYaml, session.editedYaml),
    [session.liveYaml, session.editedYaml],
  )
  const unchanged = parts.every((p) => !p.added && !p.removed)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-3">
      {session.liveYamlChangedSinceReview && (
        <p
          data-testid="yaml-diff-stale"
          className="flex items-center gap-2 border border-status-warn/50 bg-status-warn/10 px-2 py-1.5 text-xs text-status-warn"
        >
          The live object changed since you last reviewed this diff. Re-review before applying.
        </p>
      )}
      {unchanged ? (
        <p className="text-xs text-muted-foreground">No changes.</p>
      ) : (
        <pre
          data-testid="yaml-diff"
          className="min-w-0 flex-1 overflow-auto p-2 font-mono text-xs leading-relaxed whitespace-pre-wrap"
        >
          {parts.map((part, i) => (
            <span
              key={i}
              data-testid={part.added ? 'diff-added' : part.removed ? 'diff-removed' : undefined}
              className={cn(
                'block',
                part.added && 'bg-status-ok/15 text-status-ok',
                part.removed && 'bg-destructive/15 text-destructive line-through',
              )}
            >
              {part.value
                .replace(/\n$/, '')
                .split('\n')
                .map((line, j) => (
                  <span key={j} className="block">
                    {part.added ? '+ ' : part.removed ? '- ' : '  '}
                    {line}
                  </span>
                ))}
            </span>
          ))}
        </pre>
      )}
    </div>
  )
}
