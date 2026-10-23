/** 033 US1: top-bar identity chip. Collapses to an avatar on narrow widths; opens the identity dialog. */
import { UserCircleIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@companion/ui'
import { useStore } from '@/store'

import { ContextAuth } from './ContextAuth'
import { IssuerSettings } from './IssuerSettings'
import { isSignedIn } from './state'

export function IdentityChip({ compact }: { compact: boolean }) {
  const [open, setOpen] = useState(false)
  const identities = useStore((s) => s.identity.identities)
  const active = Object.values(identities).find(isSignedIn)
  const label = active ? active.username : 'Sign in'

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size={compact ? 'icon' : 'sm'}
          aria-label={active ? `Signed in as ${active.username}` : 'Identity'}
          data-testid="identity-chip"
          data-signed-in={active ? 'true' : 'false'}
          className="min-h-[var(--touch-target)] min-w-[var(--touch-target)] shrink-0"
        >
          <UserCircleIcon />
          {!compact && <span className="max-w-32 truncate text-xs">{label}</span>}
        </Button>
      </DialogTrigger>
      <DialogContent
        className="max-h-[85vh] overflow-y-auto sm:max-w-2xl"
        data-testid="identity-dialog"
      >
        <DialogHeader>
          <DialogTitle>Identity</DialogTitle>
          <DialogDescription>OIDC sign-in for your clusters.</DialogDescription>
        </DialogHeader>
        <IssuerSettings />
        <h3 className="font-display text-xs font-semibold tracking-wide uppercase">
          Context authentication
        </h3>
        <ContextAuth />
      </DialogContent>
    </Dialog>
  )
}
