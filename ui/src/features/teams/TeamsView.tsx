/**
 * 033 US3: Teams view. Everyone sees their own IdP groups (from the verified ID token claims);
 * Fathom admins additionally see the IdP directory (read via the backend-only, read-only admin
 * API) and manage advisory group -> shared-profile links. All testids are prefixed `teams-`.
 */
import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useStore } from '@/store'

import { Disconnected } from '../../components/shell/PaneStates'
import { isFathomAdmin, isSignedIn } from '../identity/state'

const ROW_HEIGHT = 32

export function TeamsView() {
  const send = useStore((s) => s.send)
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const identities = useStore((s) => s.identity.identities)
  const roleMap = useStore((s) => s.identity.config.role_map)
  const directory = useStore((s) => s.teams.directory)
  const adminKey = useStore((s) => s.teams.adminKey)
  const links = useStore((s) => s.teams.links)
  const adopted = useStore((s) => s.team.adopted)
  const available = useStore((s) => s.team.available)

  const me = Object.values(identities).find(isSignedIn)
  const issuerId = me?.issuer_id
  const isAdmin = isFathomAdmin(me, roleMap)
  const snapshot = issuerId ? directory[issuerId] : undefined

  const [keyInput, setKeyInput] = useState('')
  const [linkGroup, setLinkGroup] = useState('')
  const [linkProfile, setLinkProfile] = useState('')
  const [filter, setFilter] = useState('')

  useEffect(() => {
    send({ type: 'RefreshConnectionProfiles' })
    if (issuerId && isAdmin) send({ type: 'IdpRefreshDirectory', data: { issuer_id: issuerId } })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issuerId, isAdmin])

  const scrollRef = useRef<HTMLDivElement>(null)
  const users = (snapshot?.users ?? []).filter((u) =>
    filter.trim() === ''
      ? true
      : `${u.username} ${u.email ?? ''} ${u.groups.join(' ')}`
          .toLowerCase()
          .includes(filter.trim().toLowerCase()),
  )
  const virtualizer = useVirtualizer({
    count: users.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  })

  if (disconnected) return <Disconnected />
  if (!me || !issuerId) {
    return (
      <div data-testid="teams-view" className="p-4 text-xs text-muted-foreground">
        <p data-testid="teams-signed-out">Sign in with your team IdP to see your teams.</p>
      </div>
    )
  }

  const profiles = [...adopted.map((a) => a.profile), ...available]
  const canLink = linkGroup !== '' && linkProfile !== ''

  return (
    <div data-testid="teams-view" className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4">
      <section className="flex flex-col gap-1">
        <h2 className="text-xs font-semibold tracking-wider uppercase">Your teams</h2>
        {me.groups.length === 0 ? (
          <p data-testid="teams-no-groups" className="text-xs text-muted-foreground">
            You are not in any IdP group, so you have no Fathom-level team access.
          </p>
        ) : (
          <ul data-testid="teams-own-groups" className="flex flex-wrap gap-1">
            {me.groups.map((g) => (
              <li key={g} className="border border-border px-2 py-0.5 font-mono text-xs">
                {g}
              </li>
            ))}
          </ul>
        )}
      </section>

      {isAdmin && (
        <>
          <section className="flex flex-col gap-2" data-testid="teams-admin">
            <h2 className="text-xs font-semibold tracking-wider uppercase">IdP directory</h2>
            <div className="flex gap-2">
              <Input
                type="password"
                autoComplete="off"
                data-testid="teams-admin-key-input"
                placeholder={
                  adminKey[issuerId]
                    ? 'Admin API key stored (enter a new one to replace)'
                    : 'Read-only admin API key (name$secret)'
                }
                value={keyInput}
                onChange={(e) => setKeyInput(e.target.value)}
              />
              <Button
                size="sm"
                data-testid="teams-admin-key-save"
                disabled={keyInput.trim() === ''}
                onClick={() => {
                  send({
                    type: 'IdpSetAdminKey',
                    data: { issuer_id: issuerId, api_key: keyInput.trim() },
                  })
                  setKeyInput('')
                  send({ type: 'IdpRefreshDirectory', data: { issuer_id: issuerId } })
                }}
              >
                Save key
              </Button>
              <Button
                size="sm"
                variant="outline"
                data-testid="teams-directory-refresh"
                onClick={() => send({ type: 'IdpRefreshDirectory', data: { issuer_id: issuerId } })}
              >
                Refresh
              </Button>
            </div>
            {snapshot?.error && (
              <p
                role="status"
                data-testid="teams-idp-unavailable"
                className="border border-status-warn/50 px-2 py-1 text-xs text-status-warn"
              >
                IdP unavailable: {snapshot.error}
                {snapshot.fetched_at > 0 ? ' Showing the last successful read.' : ''}
              </p>
            )}
            {snapshot && snapshot.groups.length > 0 && (
              <ul data-testid="teams-directory-groups" className="flex flex-wrap gap-1">
                {snapshot.groups.map((g) => (
                  <li key={g.name} className="border border-border px-2 py-0.5 text-xs">
                    <span className="font-mono">{g.name}</span>{' '}
                    <span className="text-muted-foreground">{g.member_count}</span>
                  </li>
                ))}
              </ul>
            )}
            <Input
              data-testid="teams-user-filter"
              placeholder="Filter users"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
            <div
              ref={scrollRef}
              data-testid="teams-directory-users"
              data-user-count={users.length}
              className="h-64 overflow-auto border border-border"
            >
              <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
                {virtualizer.getVirtualItems().map((v) => {
                  const u = users[v.index]
                  if (!u) return null
                  return (
                    <div
                      key={u.sub}
                      data-testid={`teams-user-${u.username}`}
                      className="absolute flex w-full items-center gap-3 px-2 text-xs"
                      style={{ height: ROW_HEIGHT, transform: `translateY(${v.start}px)` }}
                    >
                      <span className="min-w-0 shrink-0 basis-40 truncate font-semibold">
                        {u.username}
                        {!u.enabled && ' (disabled)'}
                      </span>
                      <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
                        {u.groups.join(', ')}
                      </span>
                      <span className="hidden min-w-0 truncate text-muted-foreground md:block md:basis-48">
                        {u.email ?? ''}
                      </span>
                    </div>
                  )
                })}
              </div>
            </div>
          </section>

          <section className="flex flex-col gap-2" data-testid="teams-links">
            <h2 className="text-xs font-semibold tracking-wider uppercase">
              Group to profile links (advisory)
            </h2>
            {links.length === 0 && (
              <p className="text-xs text-muted-foreground">No links: every profile is visible.</p>
            )}
            {links.map((l) => (
              <div
                key={`${l.group}/${l.profile_id}`}
                data-testid={`teams-link-${l.group}-${l.profile_id}`}
                className="flex items-center justify-between gap-2 text-xs"
              >
                <span className="font-mono">
                  {l.group} {'->'} {l.profile_id}
                </span>
                <Button
                  size="xs"
                  variant="ghost"
                  data-testid={`teams-unlink-${l.group}-${l.profile_id}`}
                  onClick={() =>
                    send({
                      type: 'UnlinkTeamProfile',
                      data: { group: l.group, profile_id: l.profile_id },
                    })
                  }
                >
                  Unlink
                </Button>
              </div>
            ))}
            <div className="flex flex-wrap gap-2">
              <select
                data-testid="teams-link-group"
                className="border border-border bg-background px-2 py-1 text-xs"
                value={linkGroup}
                onChange={(e) => setLinkGroup(e.target.value)}
              >
                <option value="">Group</option>
                {(snapshot?.groups ?? []).map((g) => (
                  <option key={g.name} value={g.name}>
                    {g.name}
                  </option>
                ))}
              </select>
              <select
                data-testid="teams-link-profile"
                className="border border-border bg-background px-2 py-1 text-xs"
                value={linkProfile}
                onChange={(e) => setLinkProfile(e.target.value)}
              >
                <option value="">Profile</option>
                {profiles.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.contextAlias}
                  </option>
                ))}
              </select>
              <Button
                size="sm"
                data-testid="teams-link-add"
                disabled={!canLink}
                onClick={() => {
                  send({
                    type: 'LinkTeamProfile',
                    data: { group: linkGroup, profile_id: linkProfile },
                  })
                  setLinkGroup('')
                  setLinkProfile('')
                }}
              >
                Link
              </Button>
            </div>
          </section>
        </>
      )}
    </div>
  )
}
