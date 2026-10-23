/**
 * 043 US3: the header "Settings" menu. It sits immediately left of the sign-in chip and lists
 * the same pages as the navigator's Settings group; each item opens its page in the list pane.
 * On phone the header has no room for it, so `OverflowMenu` renders `SettingsMenuItems` instead.
 */
import { GearSixIcon } from '@phosphor-icons/react'

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@companion/ui'
import { useStore } from '@/store'

import { settingsItems } from './settings-items'

/** The menu items, shared by the header menu and the phone overflow menu. */
export function SettingsMenuItems() {
  const navigate = useStore((s) => s.navigate)
  const hosted = useStore((s) => s.hosted.mode === 'hosted')
  return (
    <>
      {settingsItems(hosted).map((item) => (
        <DropdownMenuItem
          key={item.section}
          data-testid={`settings-menu-${item.section.replace(/\//g, '-')}`}
          onSelect={() => navigate(item.section)}
        >
          <item.icon aria-hidden="true" />
          {item.label}
        </DropdownMenuItem>
      ))}
    </>
  )
}

export function SettingsMenu() {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          data-testid="settings-menu"
          className="h-[var(--touch-target)] shrink-0 gap-1.5 px-2"
        >
          <GearSixIcon aria-hidden="true" data-icon="inline-start" />
          Settings
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-48" aria-label="Settings">
        <SettingsMenuItems />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
