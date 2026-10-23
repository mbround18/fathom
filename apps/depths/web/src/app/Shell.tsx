import { MoonIcon, SignOutIcon, SunIcon } from '@phosphor-icons/react'
import { BrandMark } from '@companion/theme'
import { Button } from '@companion/ui'
import type { ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'

import { Link } from './Link'
import { useSession } from './session-store'
import { useTheme } from './theme-store'

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

/** The signed-in frame: a dense top bar, then the routed page. Mobile first. */
export function Shell({ children }: { children: ReactNode }) {
  const me = useSession((s) => s.me)
  const connection = useSession((s) => s.connection)
  const signOut = useSession((s) => s.signOut)
  const mode = useTheme((s) => s.theme.mode)
  const toggleMode = useTheme((s) => s.toggleMode)

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-2">
        <Link to="/" aria-label="Stacks" className="flex items-center">
          <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-sm" />
        </Link>
        <span className="flex-1" />
        {connection !== 'open' && (
          <span
            role="status"
            data-connection={connection}
            className="text-xs text-status-warn"
          >
            {CONNECTION_LABEL[connection]}…
          </span>
        )}
        {me && (
          <span
            data-testid="whoami"
            className="hidden max-w-40 truncate text-xs text-muted-foreground sm:inline"
          >
            {me.identity.username}
          </span>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={mode === 'dark' ? 'Use light theme' : 'Use dark theme'}
          onClick={toggleMode}
        >
          {mode === 'dark' ? <SunIcon /> : <MoonIcon />}
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Sign out" onClick={() => void signOut()}>
          <SignOutIcon />
        </Button>
      </header>
      <main className="min-h-0 flex-1 overflow-auto">{children}</main>
    </div>
  )
}
