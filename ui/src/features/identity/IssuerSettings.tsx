/**
 * 033 US1, rebuilt by 045 US5: the issuer list, and the guided path as the only way in or out of
 * it. The flat six-field form this screen used to carry is gone (T053/FR-018): it asked for an
 * issuer URL, a client id and two claim names with nothing to check them against, which is the
 * problem the guided path exists to solve. The list, the per-issuer sign-in controls and the
 * role-map editor stay.
 *
 * One mount of `GuidedIssuerPath` serves every surface this component appears on — the identity
 * chip's dialog, Settings › Sign-in and the first-run setup step all render `IssuerSettings`, so
 * the path comes with it, as a Sheet layered over whatever opened it (FR-029).
 */
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import type { OidcIssuerConfig } from '@/protocol/types'
import { useStore } from '@/store'

import { GuidedIssuerPath } from './GuidedIssuerPath'
import { RoleMapEditor } from './RoleMapEditor'
import { SignInPanel } from './SignInPanel'

export function IssuerSettings() {
  const send = useStore((s) => s.send)
  const issuers = useStore((s) => s.identity.config.issuers)
  const origins = useStore((s) => s.identity.origins)
  // `undefined` is closed; `{}` adds a new issuer and `{ issuer }` reviews an existing one. Both
  // are the same component on the same draft machine (FR-016).
  const [path, setPath] = useState<{ issuer?: OidcIssuerConfig } | undefined>(undefined)

  return (
    <div className="flex flex-col gap-4" data-testid="oidc-issuer-settings">
      <div>
        <Button size="sm" data-testid="oidc-guided-add" onClick={() => setPath({})}>
          Add issuer with the guided path
        </Button>
      </div>
      <GuidedIssuerPath
        open={path !== undefined}
        onOpenChange={(open) => setPath(open ? (path ?? {}) : undefined)}
        issuer={path?.issuer}
        origin={path?.issuer ? origins[path.issuer.id]?.origin : undefined}
      />
      {issuers.length === 0 && (
        <p className="text-xs text-muted-foreground" data-testid="oidc-no-issuers">
          No OIDC issuers configured.
        </p>
      )}
      {issuers.map((issuer) => {
        // 045 FR-032: a seeded issuer is the server's, not this screen's. It is reviewable like any
        // other, and the only place it changes is the object it was seeded from.
        const seed = origins[issuer.id]
        const seeded = seed?.origin === 'SeededFromEnvironment'
        return (
          <section
            key={issuer.id}
            className="border border-border p-2"
            data-testid={`oidc-issuer-${issuer.id}`}
          >
            <div className="mb-2 flex items-center justify-between gap-2 text-xs">
              <span className="font-semibold">{issuer.id}</span>
              <span className="min-w-0 truncate font-mono text-muted-foreground">
                {issuer.issuer_url}
              </span>
              <Button
                size="xs"
                variant="ghost"
                data-testid={`oidc-review-${issuer.id}`}
                onClick={() => setPath({ issuer })}
              >
                {seeded ? 'Review' : 'Review and edit'}
              </Button>
              {!seeded && (
                <Button
                  size="xs"
                  variant="ghost"
                  data-testid={`oidc-delete-${issuer.id}`}
                  onClick={() => send({ type: 'OidcDeleteIssuer', data: { issuer_id: issuer.id } })}
                >
                  Remove
                </Button>
              )}
            </div>
            {seeded && (
              <p
                className="mb-2 text-xs text-muted-foreground"
                data-testid={`oidc-issuer-origin-${issuer.id}`}
              >
                This issuer is seeded from the server environment and cannot be changed or removed
                here. It is set in <span className="font-mono break-all">{seed.source}</span>.
              </p>
            )}
            <SignInPanel issuer={issuer} />
          </section>
        )
      })}

      <RoleMapEditor />
    </div>
  )
}
