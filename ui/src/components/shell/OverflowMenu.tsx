/**
 * 032-ui-shell-a11y: top-bar overflow menu. Settings-like destinations always live here; at
 * phone width Fleet and Ask AI fold in too (the bar has no room for their buttons).
 */
import {
  ClipboardTextIcon,
  DotsThreeVerticalIcon,
  GearSixIcon,
  SparkleIcon,
  SquaresFourIcon,
  UsersThreeIcon,
} from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useStore } from '@/store'

import { useAssistantConfigured } from './AskAi'

export function OverflowMenu({
  folded,
  onOpenFleet,
}: {
  /** Phone: Fleet and Ask AI live in the menu instead of the bar. */
  folded: boolean
  onOpenFleet: () => void
}) {
  const navigate = useStore((s) => s.navigate)
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
          </>
        )}
        <DropdownMenuItem onSelect={() => navigate('integrations/cluster-settings')}>
          <GearSixIcon aria-hidden="true" />
          Settings
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => navigate('audit-log')}>
          <ClipboardTextIcon aria-hidden="true" />
          Audit log
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => navigate('team/profiles')}>
          <UsersThreeIcon aria-hidden="true" />
          Connection profiles
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
