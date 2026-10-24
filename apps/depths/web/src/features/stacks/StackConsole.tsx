/**
 * The stack page's live log console: one container's logs, or the whole stack's, streamed while the
 * page is open and closed (`CloseStream`) when it closes or the source changes. It renders the log
 * tab's body, so search, pause and follow work as they do in a tab; the buttons pop the source out
 * into tabs. Lazy-loaded by the stack page, so the log view loads only with it.
 */
import { ChartBarIcon, ScrollIcon, TerminalWindowIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useEffect, useId } from 'react'

import { useSession } from '@/app/session-store'
import { LogsTab } from '@/features/tabs/LogsTab'
import { useTabs } from '@/features/tabs/store'
import { closeConsole, openConsole } from '@/features/tabs/streams'
import type { LogTarget } from '@/protocol/generated'

export interface ConsoleSource {
  /** What the console streams. */
  target: LogTarget
  /** How it is named on screen: a container's name, or "all containers". */
  label: string
  /** The container, when it is one (a shell and its stats need one). */
  container: string | null
}

export default function StackConsole({ stack, source }: { stack: string; source: ConsoleSource }) {
  // `console:` ids are never a tab's; one per mounted console, the source in it so a change of
  // source is a new stream with its own lines and search.
  const key = JSON.stringify(source.target)
  const id = `console:${useId()}:${key}`
  useEffect(() => {
    openConsole(id, JSON.parse(key) as LogTarget)
    return () => closeConsole(id)
  }, [id, key])

  const openTab = useTabs((s) => s.openTab)
  const shellOn = useSession((s) => s.me?.features.shell ?? false)
  const { target, label, container } = source
  const statsTarget = container === null ? { kind: 'stack' as const, stack } : { kind: 'container' as const, container }

  return (
    <section
      data-console
      data-console-target={container ?? `stack:${stack}`}
      aria-label={`Logs of ${label}`}
      className="flex h-full min-h-0 min-w-0 flex-col border border-border bg-card"
    >
      <header className="flex min-w-0 flex-wrap items-center gap-1 border-b border-border px-2 py-1">
        <h2 className="min-w-0 flex-1 truncate font-display text-xs font-semibold tracking-wider text-muted-foreground uppercase">
          Logs <span className="font-mono normal-case text-foreground">{label}</span>
        </h2>
        <Button
          variant="ghost"
          size="xs"
          data-action="console-pop-logs"
          aria-label={`Open the logs of ${label} in a tab`}
          title="Open these logs in a tab"
          onClick={() => openTab({ kind: 'logs', target })}
        >
          <ScrollIcon aria-hidden /> Tab
        </Button>
        {container !== null && shellOn && (
          <Button
            variant="ghost"
            size="xs"
            data-action="console-pop-shell"
            aria-label={`Open a shell in ${container} in a tab`}
            title="Open a shell in a tab"
            onClick={() => openTab({ kind: 'shell', container })}
          >
            <TerminalWindowIcon aria-hidden /> Shell
          </Button>
        )}
        <Button
          variant="ghost"
          size="xs"
          data-action="console-pop-stats"
          aria-label={`Open the stats of ${label} in a tab`}
          title="Open stats in a tab"
          onClick={() => openTab({ kind: 'stats', target: statsTarget })}
        >
          <ChartBarIcon aria-hidden /> Stats
        </Button>
      </header>
      <div className="min-h-0 flex-1">
        <LogsTab key={id} tab={{ id, kind: 'logs', target }} />
      </div>
    </section>
  )
}
