/**
 * Connection Profiles panel (014-teams-organizations, Story 1): shareable, non-secret
 * cluster-connection metadata (context alias, API server endpoint, default namespace) via a
 * self-hosted shared config store the operator's install points at (FR-001..FR-005, FR-015).
 *
 * The store never carries credentials — `ConnectionProfile` has no such field on the wire, and
 * the backend's `scan_fields_for_secrets` heuristic rejects any free-text field that looks like
 * one (`TeamPublishRejected { rejection: { kind: 'suspectedSecret', .. } }`).
 *
 * All new testids are prefixed `teams-`.
 */
import { useEffect, useState } from 'react'

import { isSignedIn } from '@/features/identity/state'
import { filterProfiles } from '@/features/teams/filter'
import { Button, Input } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { Disconnected, Empty } from '@companion/terminal'

export function TeamProfilesPanel() {
  const request = useStore((s) => s.request)
  const send = useStore((s) => s.send)
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const storeStatus = useStore((s) => s.team.storeStatus)
  const adopted = useStore((s) => s.team.adopted)
  const allAvailable = useStore((s) => s.team.available)
  const links = useStore((s) => s.teams.links)
  const identities = useStore((s) => s.identity.identities)
  const [showAll, setShowAll] = useState(false)
  const myGroups = Object.values(identities)
    .filter(isSignedIn)
    .flatMap((i) => i.groups)
  const signedIn = Object.values(identities).some(isSignedIn)
  const filtering = signedIn && links.length > 0 && !showAll
  const available = filtering ? filterProfiles(allAvailable, links, myGroups) : allAvailable
  const hiddenCount = allAvailable.length - available.length
  const rejection = useStore((s) => s.team.rejection)

  const [storePathInput, setStorePathInput] = useState('')
  const [storePathPrefilled, setStorePathPrefilled] = useState(false)
  const [savingPath, setSavingPath] = useState(false)

  const [contextAlias, setContextAlias] = useState('')
  const [apiServer, setApiServer] = useState('')
  const [defaultNamespace, setDefaultNamespace] = useState('')
  const [publishing, setPublishing] = useState(false)
  const [publishError, setPublishError] = useState<string | undefined>(undefined)

  const [busyProfileId, setBusyProfileId] = useState<string | undefined>(undefined)

  useEffect(() => {
    send({ type: 'RefreshTeamStoreStatus' })
    send({ type: 'RefreshConnectionProfiles' })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  if (!storePathPrefilled && storeStatus?.storePath !== undefined) {
    setStorePathPrefilled(true)
    setStorePathInput(storeStatus.storePath ?? '')
  }

  if (disconnected) return <Disconnected />

  const saveStorePath = () => {
    if (savingPath) return
    setSavingPath(true)
    void (async () => {
      try {
        const requestId = await request({
          type: 'SetTeamStorePath',
          data: { path: storePathInput.trim() === '' ? null : storePathInput.trim() },
        })
        await waitForCommandResult(requestId)
        send({ type: 'RefreshTeamStoreStatus' })
        send({ type: 'RefreshConnectionProfiles' })
      } finally {
        setSavingPath(false)
      }
    })()
  }

  const publish = () => {
    if (publishing || contextAlias.trim() === '' || apiServer.trim() === '') return
    setPublishing(true)
    setPublishError(undefined)
    void (async () => {
      try {
        const requestId = await request({
          type: 'PublishConnectionProfile',
          data: {
            context_alias: contextAlias.trim(),
            api_server: apiServer.trim(),
            default_namespace: defaultNamespace.trim() === '' ? null : defaultNamespace.trim(),
          },
        })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') setPublishError('The backend did not respond in time.')
        else if (!result.ok) setPublishError(result.error ?? 'Publishing the profile failed.')
        else {
          setContextAlias('')
          setApiServer('')
          setDefaultNamespace('')
        }
      } catch (err) {
        setPublishError(err instanceof Error ? err.message : String(err))
      } finally {
        setPublishing(false)
      }
    })()
  }

  const adopt = (profileId: string) => {
    setBusyProfileId(profileId)
    send({ type: 'AdoptConnectionProfile', data: { profile_id: profileId } })
    window.setTimeout(() => setBusyProfileId(undefined), 500)
  }
  const remove = (profileId: string) => {
    setBusyProfileId(profileId)
    send({ type: 'RemoveConnectionProfile', data: { profile_id: profileId } })
    window.setTimeout(() => setBusyProfileId(undefined), 500)
  }

  return (
    <div
      data-testid="teams-profiles-panel"
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4"
    >
      <fieldset className="flex flex-col gap-2">
        <legend className="text-xs font-semibold tracking-wider uppercase">
          Shared team store
        </legend>
        <div className="flex gap-2">
          <Input
            data-testid="teams-store-path-input"
            placeholder="/path/to/shared/store or a mounted network share"
            value={storePathInput}
            onChange={(e) => setStorePathInput(e.target.value)}
          />
          <Button
            type="button"
            size="sm"
            data-testid="teams-save-store-path"
            onClick={saveStorePath}
            disabled={savingPath}
          >
            {savingPath ? 'Saving…' : 'Save'}
          </Button>
        </div>
        <p
          data-testid="teams-store-status"
          data-reachable={storeStatus?.reachable ?? false}
          className="text-xs text-muted-foreground"
        >
          {storeStatus === undefined
            ? 'Status unknown.'
            : storeStatus.storePath === undefined || storeStatus.storePath === null
              ? 'No shared store configured yet.'
              : storeStatus.reachable
                ? `Reachable: ${storeStatus.storePath}`
                : `Unreachable: ${storeStatus.error ?? storeStatus.storePath}`}
        </p>
      </fieldset>

      {rejection !== undefined && (
        <p
          role="alert"
          data-testid="teams-publish-rejection"
          className="border border-status-err/50 px-2 py-1 text-xs text-status-err"
        >
          {rejection.kind === 'suspectedSecret'
            ? `Publish refused: field "${rejection.field}" ${rejection.reason}`
            : rejection.kind === 'conflict'
              ? 'Publish refused: someone else published a newer version first. Refresh and retry.'
              : rejection.kind === 'roleForbidden'
                ? `Not permitted: ${rejection.reason}`
                : 'Publish refused: the shared team store is unreachable.'}
        </p>
      )}

      <fieldset className="flex flex-col gap-2">
        <legend className="text-xs font-semibold tracking-wider uppercase">
          Share this connection
        </legend>
        <Input
          data-testid="teams-publish-context-alias"
          placeholder="Context alias"
          value={contextAlias}
          onChange={(e) => setContextAlias(e.target.value)}
        />
        <Input
          data-testid="teams-publish-api-server"
          placeholder="API server (e.g. https://cluster.example.com:6443)"
          value={apiServer}
          onChange={(e) => setApiServer(e.target.value)}
        />
        <Input
          data-testid="teams-publish-default-namespace"
          placeholder="Default namespace (optional)"
          value={defaultNamespace}
          onChange={(e) => setDefaultNamespace(e.target.value)}
        />
        {publishError !== undefined && (
          <p role="alert" data-testid="teams-publish-error" className="text-xs text-status-err">
            {publishError}
          </p>
        )}
        <Gated verb="get" resource="namespaces">
          <Button
            type="button"
            data-testid="teams-publish-profile"
            onClick={publish}
            disabled={publishing}
          >
            {publishing ? 'Publishing…' : 'Publish'}
          </Button>
        </Gated>
      </fieldset>

      <section className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold tracking-wider uppercase">Adopted</h3>
        {adopted.length === 0 ? (
          <Empty title="No adopted profiles" detail="Adopt one from Available below." />
        ) : (
          <ul data-testid="teams-adopted-list" className="flex flex-col gap-1">
            {adopted.map((a) => (
              <li
                key={a.profile.id}
                data-testid="teams-adopted-row"
                className="flex items-center justify-between border border-border px-2 py-1 text-xs"
              >
                <span>
                  <strong>{a.profile.contextAlias}</strong> — {a.profile.apiServer}
                  {a.profile.removed && (
                    <span
                      data-testid="teams-profile-no-longer-shared"
                      className="ml-2 text-status-err"
                    >
                      no longer shared
                    </span>
                  )}
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  data-testid="teams-remove-profile"
                  onClick={() => remove(a.profile.id)}
                  disabled={busyProfileId === a.profile.id}
                >
                  Remove
                </Button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold tracking-wider uppercase">Available</h3>
        {signedIn && links.length > 0 && (
          <p
            data-testid="teams-advisory-filter"
            className="flex items-center gap-2 text-xs text-muted-foreground"
          >
            <span>
              Filtered by your IdP groups (advisory, not access control).
              {filtering && hiddenCount > 0 ? ` ${hiddenCount} hidden.` : ''}
            </span>
            <Button
              size="xs"
              variant="ghost"
              data-testid="teams-advisory-toggle"
              onClick={() => setShowAll((v) => !v)}
            >
              {showAll ? 'Filter by my groups' : 'Show all'}
            </Button>
          </p>
        )}
        {available.length === 0 ? (
          <Empty title="No profiles to adopt" />
        ) : (
          <ul data-testid="teams-available-list" className="flex flex-col gap-1">
            {available.map((p) => (
              <li
                key={p.id}
                data-testid="teams-available-row"
                className="flex items-center justify-between border border-border px-2 py-1 text-xs"
              >
                <span>
                  <strong>{p.contextAlias}</strong> — {p.apiServer} (by {p.sharedBy})
                </span>
                <Button
                  type="button"
                  size="sm"
                  data-testid="teams-adopt-profile"
                  onClick={() => adopt(p.id)}
                  disabled={busyProfileId === p.id}
                >
                  Adopt
                </Button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  )
}
