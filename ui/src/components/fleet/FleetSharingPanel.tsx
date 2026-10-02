/**
 * 046-fleet-model US3 (T043): publish the fleet's labels and groups to the 014 shared store, adopt
 * what a colleague published, and — the part that matters — never tell the operator their adopted
 * copy is gone when it is not.
 *
 * Two states here are the ones where the honest message and the convenient message differ:
 *
 * - **The store is unreachable.** The convenient rendering is "not shared" or an empty list. The
 *   truth is that the store could not be reached and every adopted definition is still on this
 *   machine and still in use (FR-014). So the row stays, flagged as no longer following, and the
 *   store's own error is shown verbatim beside it.
 * - **The definition was removed upstream.** The row is kept, flagged `no longer shared`, and only
 *   an explicit operator action removes it (FR-014, Constitution Principle VI).
 *
 * The panel composes no message the backend already wrote: `FleetStoreUnavailable.reason` renders
 * verbatim, as does every selector in a shared definition.
 */
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { notFollowingReason, sharedDefinitionRows } from '@/resources/fleet'
import { useStore } from '@/store'

export function FleetSharingPanel() {
  const definitions = useStore((s) => s.session.fleetDefinitions)
  const adopted = useStore((s) => s.session.fleetAdopted)
  const storeError = useStore((s) => s.session.fleetStoreError)
  const send = useStore((s) => s.send)

  const [id, setId] = useState('')

  useEffect(() => {
    // Reading the shared store starts no poller and no watch (FR-015).
    send({ type: 'RefreshSharedFleetDefinitions' })
  }, [send])

  const rows = sharedDefinitionRows(definitions, adopted, storeError !== null)

  return (
    <section
      data-testid="fleet-sharing"
      className="flex min-w-0 flex-col gap-2 rounded-md border border-border/60 p-2"
    >
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-medium">Shared definitions</h3>
        <span className="text-xs text-muted-foreground">
          Labels and groups, as your team shares them
        </span>
        <button
          type="button"
          data-testid="fleet-sharing-refresh"
          className="ml-auto text-xs text-muted-foreground hover:text-foreground"
          onClick={() => send({ type: 'RefreshSharedFleetDefinitions' })}
        >
          Refresh
        </button>
      </header>

      {/* FR-014: the store being unreachable changes what is being followed, not what is held. */}
      {storeError === null ? null : (
        <p
          data-testid="fleet-store-unavailable"
          className="flex min-w-0 flex-col gap-1 rounded border border-amber-500/60 bg-amber-500/10 p-2 text-xs text-amber-300"
        >
          <span>
            The shared store could not be reached, so nothing is following it right now. Every
            definition you adopted is still here and still in use.
          </span>
          {/* The store's own message (Principle I). */}
          <span
            data-testid="fleet-store-unavailable-reason"
            className="font-mono break-words text-muted-foreground"
          >
            {storeError}
          </span>
        </p>
      )}

      <span className="flex min-w-0 flex-wrap items-center gap-1">
        <Input
          data-testid="fleet-share-id-input"
          aria-label="Shared definition id to publish as"
          placeholder="platform-prod"
          value={id}
          onChange={(e) => setId(e.target.value)}
          className="h-8 min-w-0 flex-1 font-mono text-sm sm:max-w-xs"
        />
        <Button
          data-testid="fleet-share-publish"
          size="sm"
          variant="outline"
          disabled={id.trim() === ''}
          onClick={() => send({ type: 'PublishFleetDefinition', data: { id } })}
        >
          Publish
        </Button>
      </span>

      {rows.length === 0 ? (
        <p data-testid="fleet-sharing-none" className="text-xs text-muted-foreground">
          {storeError === null
            ? 'The store holds no fleet definitions yet.'
            : 'Nothing has been adopted on this machine yet.'}
        </p>
      ) : (
        rows.map((row) => (
          <article
            key={row.definition.id}
            data-testid="fleet-shared-definition"
            data-definition={row.definition.id}
            data-adopted={row.adopted ? 'yes' : 'no'}
            data-following={row.following ? 'yes' : 'no'}
            className="flex min-w-0 flex-col gap-1 rounded border border-border/60 p-2"
          >
            <header className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-mono text-sm">{row.definition.id}</span>
              <span className="text-muted-foreground">
                rev {row.definition.rev} · shared by {row.definition.shared_by}
              </span>
              <span className="ml-auto flex items-center gap-1">
                <Button
                  data-testid="fleet-definition-adopt"
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    send({ type: 'AdoptFleetDefinition', data: { id: row.definition.id } })
                  }
                >
                  {row.adopted ? 'Adopt again' : 'Adopt'}
                </Button>
                <Button
                  data-testid="fleet-definition-remove"
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    send({ type: 'RemoveFleetDefinition', data: { id: row.definition.id } })
                  }
                >
                  Remove
                </Button>
              </span>
            </header>

            {/* Kept, flagged, and removed only on purpose (FR-014). */}
            {row.adopted && !row.following ? (
              <p
                data-testid="fleet-definition-not-following"
                className="text-xs break-words text-amber-300"
              >
                {notFollowingReason(row)}
              </p>
            ) : null}

            <ul className="flex min-w-0 flex-col gap-0.5">
              {Object.entries(row.definition.groups).map(([name, selector]) => (
                <li
                  key={name}
                  data-testid="fleet-shared-group"
                  data-group={name}
                  className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs"
                >
                  <span className="font-medium">{name}</span>
                  {/* The author's expression, verbatim as they typed it. */}
                  <code className="truncate rounded bg-muted/40 px-1 py-0.5 font-mono text-muted-foreground">
                    {selector}
                  </code>
                </li>
              ))}
            </ul>

            <ul className="flex min-w-0 flex-col gap-0.5">
              {Object.entries(row.definition.members).map(([context, share]) => (
                <li
                  key={context}
                  data-testid="fleet-shared-member"
                  data-context-name={context}
                  className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground"
                >
                  {/* Both names, always — the context name is the key (FR-002). */}
                  {share.display_name == null ? null : (
                    <span className="text-foreground">{share.display_name}</span>
                  )}
                  <span className="truncate font-mono">{context}</span>
                  {Object.entries(share.labels).map(([k, v]) => (
                    <span
                      key={k}
                      data-testid="fleet-shared-label"
                      data-label-key={k}
                      className="rounded border border-border/60 px-1 py-0.5 font-mono"
                    >
                      {k}={v}
                    </span>
                  ))}
                </li>
              ))}
            </ul>
          </article>
        ))
      )}
    </section>
  )
}
