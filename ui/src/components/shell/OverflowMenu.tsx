/**
 * 032-ui-shell-a11y: top-bar overflow menu. At phone width Fleet, Ask AI and the Settings pages
 * fold in here (the bar has no room for their buttons). 043 US3: from tablet up the Settings
 * pages live in the header's own Settings menu instead.
 */
import { DotsThreeVerticalIcon, SparkleIcon, SquaresFourIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import { SettingsMenuItems } from '@/components/settings/SettingsMenu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useStore } from '@/store'

import { useAssistantConfigured } from './AskAi'

export function OverflowMenu({
  folded,
  onOpenFleet,
}: {
  /** Phone: Fleet, Ask AI and the Settings pages live in the menu instead of the bar. */
  folded: boolean
  onOpenFleet: () => void
}) {
  const setShell = useStore((s) => s.setShell)
  const configured = useAssistantConfigured()

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label="More actions"
          data-testid="overflow-menu"
          className="size-[var(--touch-target)] shrink-0"
        >
          <DotsThreeVerticalIcon aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-48">
        {folded && (
          <>
            <DropdownMenuItem
              onSelect={() => setShell({ askAiOpen: true, askAiSetup: !configured })}
            >
              <SparkleIcon aria-hidden="true" />
              {configured ? 'Ask AI' : 'Ask AI (set up)'}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onOpenFleet}>
              <SquaresFourIcon aria-hidden="true" />
              Fleet
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Settings</DropdownMenuLabel>
            <SettingsMenuItems />
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
