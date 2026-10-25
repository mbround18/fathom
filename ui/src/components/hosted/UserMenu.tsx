import { useState } from 'react'
import { SignOutIcon, UserIcon } from '@phosphor-icons/react'

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@companion/ui'
import type { PublicIdentity } from '@/protocol/generated/PublicIdentity'

/** Identity display + sign-out. `onSignOut` performs the CSRF-protected logout and navigation. */
export function UserMenu({
  identity,
  onSignOut,
}: {
  identity: PublicIdentity
  onSignOut: () => void | Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" aria-label={`Account menu for ${identity.username}`}>
          <UserIcon aria-hidden />
          <span className="hidden max-w-32 truncate sm:inline">{identity.username}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-56">
        <DropdownMenuLabel className="normal-case">
          <span className="block truncate text-foreground">{identity.username}</span>
          {identity.groups.length > 0 && (
            <span className="block truncate text-xs font-normal text-muted-foreground">
              {identity.groups.join(', ')}
            </span>
          )}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={busy}
          onSelect={() => {
            setBusy(true)
            void Promise.resolve(onSignOut()).finally(() => {
              setBusy(false)
            })
          }}
        >
          <SignOutIcon aria-hidden /> Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
