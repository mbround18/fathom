/**
 * Settings → Relays (spec 067 US3, T035; contracts/ui.md "Settings → Relays"): the workspace's zones
 * and relays, and Keel's relay-link key.
 *
 * - The relay-link key: the line every relay lists in `keel_relay_hub_keys`, with Copy. Instance
 *   admins also rotate, activate and retire it here. A workspace admin cannot read it, but the
 *   Register preview carries the line.
 * - Zones: create, rename, remove. Removing a zone still in use previews its blockers.
 * - Relays: register and edit (name, zone, dial address, the relay's pasted host key, and the host
 *   it runs on); the preview shows the fingerprint Keel pins. Revoke is one click to a preview with
 *   no passkey, naming the hosts whose sessions end. A quarantined relay shows why, and Release.
 *
 * Every change is a governance preview → confirm (`relays-store.ts`).
 */
import {
  ArrowCounterClockwiseIcon,
  ArrowsClockwiseIcon,
  CheckIcon,
  PencilSimpleIcon,
  PlusIcon,
  ProhibitIcon,
  TrashIcon,
  XIcon,
} from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import {
  CopyBlock,
  DataTable,
  Dialog,
  Field,
  FIELD_AREA,
  Fingerprint,
  H2,
  LoadFailed,
  LoadState,
  SectionIntro,
  Select,
  STATUS,
  type Column,
} from '@companion/ui/kit'
import { useEffect, useId, useMemo, useState, type SyntheticEvent } from 'react'

import { useSession } from '@/app/session-store'
import { useCurrentWorkspace } from '@/app/workspace-store'
import { relayTone, TONE, zoneName } from '@/features/hosts/reach'
import { useHosts } from '@/features/hosts/store'
import { canAdminister } from '@/features/hosts/workspaces'
import { useGovernance } from '@/features/teams/store'
import type { RelayInfo, RelayLinkKey, ZoneInfo } from '@/protocol/wire'

import { useRelays, validDialAddress, validHostKey, validName } from './relays-store'
import { canSeeSettings } from './tabs'

const SECTION = 'flex min-w-0 flex-col gap-3'
/** Until the relays guide lands (T049), the hosts guide covers where hosts connect from. */
const GUIDE = { guide: 'hosts' }
const HINT_ERR = 'text-xs text-status-err'

/** Keel's relay-link key: the active line to copy, and, for instance admins, rotation. */
function LinkKey() {
  const me = useSession((s) => s.me)
  const keys = useRelays((s) => s.keys)
  const loadKeys = useRelays((s) => s.loadKeys)
  const rotate = useRelays((s) => s.rotateKey)
  const activate = useRelays((s) => s.activateKey)
  const retire = useRelays((s) => s.retireKey)
  const admin = canSeeSettings(me?.instance_role)
  useEffect(() => {
    if (admin) loadKeys()
  }, [admin, loadKeys])
  const active = keys.value?.find((k) => k.state === 'active')
  const pending = keys.value?.find((k) => k.state === 'pending')
  return (
    <section aria-labelledby="relay-link-key" className={SECTION}>
      <h2 id="relay-link-key" className={H2}>
        Relay-link key
      </h2>
      <SectionIntro guide={GUIDE}>
        Keel signs in to every relay with this key. Add this to <code className="font-mono">keel_relay_hub_keys</code> on each relay.
      </SectionIntro>
      {!admin ? (
        <p data-link-key="hidden" className="text-xs text-muted-foreground">
          Only an instance admin can read it here. Registering a relay shows the line in its preview.
        </p>
      ) : keys.refusal ? (
        <LoadFailed what="the relay-link key" refusal={keys.refusal} onRetry={loadKeys} />
      ) : keys.value === null ? (
        <LoadState loading label="Loading the relay-link key…" />
      ) : (
        <>
          {active && <CopyBlock label="Relay-link key" value={active.public_line} data="relay-link-key" />}
          {pending && <PendingKey pending={pending} onActivate={activate} onRetire={retire} />}
          <KeyList keys={keys.value} />
          {!pending && (
            <div>
              <Button size="sm" variant="outline" onClick={rotate}>
                <ArrowsClockwiseIcon aria-hidden />
                Rotate
              </Button>
            </div>
          )}
        </>
      )}
    </section>
  )
}

function PendingKey({ pending, onActivate, onRetire }: { pending: RelayLinkKey; onActivate: (k: RelayLinkKey) => void; onRetire: (k: RelayLinkKey) => void }) {
  return (
    <div data-link-key="pending" className="flex min-w-0 flex-col gap-2 border border-status-warn/50 p-3">
      <p className="text-xs">
        A new key is pending. Add its line to every relay&apos;s <code className="font-mono">keel_relay_hub_keys</code>, then activate it.
        Keel keeps using the active key until then.
      </p>
      <CopyBlock label="Pending relay-link key" value={pending.public_line} data="relay-link-key-pending" />
      <div className="flex min-w-0 flex-wrap gap-2">
        <Button
          size="sm"
          onClick={() => {
            onActivate(pending)
          }}
        >
          <CheckIcon aria-hidden />
          Activate
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            onRetire(pending)
          }}
        >
          <XIcon aria-hidden />
          Retire
        </Button>
      </div>
    </div>
  )
}

function KeyList({ keys }: { keys: RelayLinkKey[] }) {
  if (keys.length < 2) return null
  return (
    <ul aria-label="Relay-link keys" className="flex min-w-0 flex-col gap-1 text-xs">
      {keys.map((k) => (
        <li key={k.fingerprint} className="flex min-w-0 flex-wrap items-center gap-2">
          <span className={`${STATUS} w-16 ${k.state === 'active' ? 'text-status-ok' : k.state === 'pending' ? 'text-status-warn' : 'text-muted-foreground'}`}>
            {k.state}
          </span>
          <Fingerprint value={k.fingerprint} label={`${k.state} key fingerprint`} />
        </li>
      ))}
    </ul>
  )
}

/** Create or rename a zone; `zone: null` creates. */
function ZoneDialog({ zone, open, onClose }: { zone: ZoneInfo | null; open: boolean; onClose: () => void }) {
  const createZone = useRelays((s) => s.createZone)
  const updateZone = useRelays((s) => s.updateZone)
  const changing = useGovernance((s) => s.change !== null)
  const [name, setName] = useState(zone?.name ?? '')
  const [description, setDescription] = useState(zone?.description ?? '')
  const nameId = useId()
  const descId = useId()
  const nameOk = validName(name)
  const changed = zone === null || name !== zone.name || description !== zone.description
  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (!nameOk || !changed) return
    if (zone) updateZone(zone, name, description, onClose)
    else createZone(name, description, onClose)
  }
  return (
    <Dialog
      open={open && !changing}
      onClose={onClose}
      size="sm"
      data-zone-dialog={zone ? 'edit' : 'create'}
      title={zone ? `Edit zone ${zone.name}` : 'Create zone'}
      description="A zone is a network Keel cannot reach straight; its hosts come in through its relays."
    >
      <form className="flex min-w-0 flex-col gap-3" onSubmit={submit}>
        <Field
          label="Name"
          htmlFor={nameId}
          hint={name !== '' && !nameOk ? <span className={HINT_ERR}>Lowercase letters, digits and dashes; not “direct”.</span> : undefined}
        >
          <Input
            id={nameId}
            value={name}
            autoComplete="off"
            spellCheck={false}
            className="font-mono"
            placeholder="lab"
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </Field>
        <Field label="Description" htmlFor={descId}>
          <textarea
            id={descId}
            value={description}
            maxLength={500}
            rows={2}
            className={FIELD_AREA}
            onChange={(e) => {
              setDescription(e.target.value)
            }}
          />
        </Field>
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={!nameOk || !changed}>
            Preview
          </Button>
        </div>
      </form>
    </Dialog>
  )
}

function Zones({ canEdit }: { canEdit: boolean }) {
  const zones = useRelays((s) => s.zones)
  const workspace = useRelays((s) => s.workspace)
  const load = useRelays((s) => s.load)
  const removeZone = useRelays((s) => s.removeZone)
  // `undefined`: closed; `null`: creating.
  const [editing, setEditing] = useState<ZoneInfo | null | undefined>(undefined)
  const columns = useMemo<Column<ZoneInfo>[]>(
    () => [
      { key: 'name', header: 'Name', className: 'w-[22%]', cell: (z) => <span className="font-mono break-all">{z.name}</span> },
      { key: 'description', header: 'Description', cell: (z) => <span className="break-words text-muted-foreground">{z.description || '—'}</span> },
      { key: 'hosts', header: 'Hosts', className: 'w-20', cell: (z) => <span className="tabular-nums">{z.hosts}</span> },
      {
        key: 'relays',
        header: 'Relays',
        className: 'w-28',
        cell: (z) =>
          z.relays === 0 ? (
            <span data-zone-no-relay className="font-medium text-status-err">
              0, no path
            </span>
          ) : (
            <span className="tabular-nums">{z.relays}</span>
          ),
      },
    ],
    [],
  )
  return (
    <section aria-labelledby="zones-title" className={SECTION}>
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <h2 id="zones-title" className={H2}>
          Zones
        </h2>
        {canEdit && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setEditing(null)
            }}
          >
            <PlusIcon aria-hidden />
            Create zone
          </Button>
        )}
      </div>
      <SectionIntro guide={GUIDE}>
        Hosts with no zone dial Keel straight (<code className="font-mono">direct</code>). Put a host in a zone from its inspector on the Hosts page.
      </SectionIntro>
      {zones.refusal && workspace ? (
        <LoadFailed
          what="zones"
          refusal={zones.refusal}
          onRetry={() => {
            load(workspace)
          }}
        />
      ) : (
        <DataTable
          label="Zones"
          columns={columns}
          rows={zones.value ?? []}
          rowKey={(z) => z.id}
          rowData={(z) => ({ 'data-zone': z.name })}
          actions={(z) =>
            canEdit && (
              <span className="flex flex-wrap justify-end gap-1">
                <Button
                  variant="ghost"
                  size="xs"
                  aria-label={`Edit zone ${z.name}`}
                  onClick={() => {
                    setEditing(z)
                  }}
                >
                  <PencilSimpleIcon aria-hidden />
                  Edit
                </Button>
                <Button
                  variant="ghost"
                  size="xs"
                  className="text-status-err"
                  aria-label={`Remove zone ${z.name}`}
                  onClick={() => {
                    removeZone(z)
                  }}
                >
                  <TrashIcon aria-hidden />
                  Remove
                </Button>
              </span>
            )
          }
          empty={
            zones.value === null ? (
              <LoadState loading label="Loading zones…" />
            ) : (
              <span className="text-muted-foreground">No zones yet. Every host dials Keel straight.</span>
            )
          }
        />
      )}
      {editing !== undefined && (
        <ZoneDialog
          key={editing?.id ?? 'new'}
          zone={editing}
          open
          onClose={() => {
            setEditing(undefined)
          }}
        />
      )}
    </section>
  )
}

/** `host:port`, wrapping only at dots and before the port, so the port never splits. */
function DialAddress({ value }: { value: string }) {
  const parts = value.split(/(?=[.:])/)
  return (
    <span className="font-mono text-xs break-normal [overflow-wrap:anywhere]">
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && <wbr />}
          {p}
        </span>
      ))}
    </span>
  )
}

/** A relay's state, and why when it is quarantined or unreachable. */
function RelayStateCell({ relay }: { relay: RelayInfo }) {
  const tone = relayTone(relay.state)
  return (
    <span className="flex min-w-0 flex-col">
      <span className={`${STATUS} ${TONE[tone]}`} data-relay-state={relay.state}>
        {relay.state}
      </span>
      {relay.last_error && (relay.state === 'quarantined' || relay.state === 'unreachable') && (
        <span data-relay-reason className="text-xs break-words text-muted-foreground">
          {relay.last_error}
        </span>
      )}
    </span>
  )
}

/** Register or edit a relay. The draft lives in the store, so it survives the preview. */
function RelayDialog() {
  const editor = useRelays((s) => s.editor)
  const zones = useRelays((s) => s.zones.value) ?? []
  const editDraft = useRelays((s) => s.editDraft)
  const close = useRelays((s) => s.closeEditor)
  const save = useRelays((s) => s.saveRelay)
  const hosts = useHosts((s) => s.hosts)
  const changing = useGovernance((s) => s.change !== null)
  const ids = { name: useId(), zone: useId(), addr: useId(), key: useId(), host: useId() }
  if (!editor) return null
  const { draft, relay } = editor
  const nameOk = validName(draft.name)
  const addrOk = validDialAddress(draft.dial_address)
  const keyOk = relay !== null && draft.host_key.trim() === '' ? true : validHostKey(draft.host_key)
  const ok = nameOk && addrOk && keyOk && draft.zone !== ''
  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (ok) save()
  }
  return (
    <Dialog
      open={!changing}
      onClose={close}
      size="md"
      data-relay-dialog={relay ? 'edit' : 'register'}
      title={relay ? `Edit relay ${relay.name}` : 'Register relay'}
      description="Keel dials the relay at its address and pins its host key. The preview shows the fingerprint it will pin."
    >
      <form className="flex min-w-0 flex-col gap-3" onSubmit={submit}>
        <div className="grid min-w-0 gap-3 sm:grid-cols-2">
          <Field
            label="Name"
            htmlFor={ids.name}
            hint={draft.name !== '' && !nameOk ? <span className={HINT_ERR}>Lowercase letters, digits and dashes.</span> : undefined}
          >
            <Input
              id={ids.name}
              value={draft.name}
              autoComplete="off"
              spellCheck={false}
              className="font-mono"
              placeholder="gw-lab"
              onChange={(e) => {
                editDraft({ name: e.target.value })
              }}
            />
          </Field>
          <Field label="Zone" htmlFor={ids.zone}>
            <Select
              id={ids.zone}
              value={draft.zone}
              onChange={(e) => {
                editDraft({ zone: e.target.value })
              }}
            >
              {zones.length === 0 && <option value="">Create a zone first</option>}
              {zones.map((z) => (
                <option key={z.id} value={z.id}>
                  {z.name}
                </option>
              ))}
            </Select>
          </Field>
        </div>
        <Field
          label="Dial address"
          htmlFor={ids.addr}
          hint={
            draft.dial_address !== '' && !addrOk ? (
              <span className={HINT_ERR}>host:port, with no scheme.</span>
            ) : (
              'Where Keel reaches the relay, as host:port.'
            )
          }
        >
          <Input
            id={ids.addr}
            value={draft.dial_address}
            autoComplete="off"
            spellCheck={false}
            className="font-mono"
            placeholder="gw.lab.example.net:7444"
            onChange={(e) => {
              editDraft({ dial_address: e.target.value })
            }}
          />
        </Field>
        <Field
          label="Host key"
          htmlFor={ids.key}
          hint={
            draft.host_key.trim() !== '' && !keyOk ? (
              <span className={HINT_ERR}>One ssh-ed25519 line, as the relay prints it.</span>
            ) : relay ? (
              <>
                Pinned: <span className="font-mono break-all">{relay.host_key_fingerprint}</span>. Paste a new key only if the relay&apos;s changed.
              </>
            ) : (
              'The relay prints its public host key when it starts. Paste the whole line.'
            )
          }
        >
          <textarea
            id={ids.key}
            value={draft.host_key}
            rows={3}
            spellCheck={false}
            className={`${FIELD_AREA} font-mono text-xs break-all`}
            placeholder="ssh-ed25519 AAAA…"
            onChange={(e) => {
              editDraft({ host_key: e.target.value })
            }}
          />
        </Field>
        <Field label="Runs on host (optional)" htmlFor={ids.host} hint="Keel warns before a run that could restart the relay under the hosts it carries.">
          <Select
            id={ids.host}
            value={draft.host ?? ''}
            onChange={(e) => {
              editDraft({ host: e.target.value === '' ? null : e.target.value })
            }}
          >
            <option value="">Not an enrolled host</option>
            {hosts
              .filter((h) => h.state !== 'revoked')
              .map((h) => (
                <option key={h.id} value={h.id}>
                  {h.inventory_name}
                </option>
              ))}
          </Select>
        </Field>
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={!ok}>
            Preview
          </Button>
        </div>
      </form>
    </Dialog>
  )
}

const NO_ZONES: ZoneInfo[] = []

function Relays({ canEdit }: { canEdit: boolean }) {
  const relays = useRelays((s) => s.relays)
  const zones = useRelays((s) => s.zones.value) ?? NO_ZONES
  const workspace = useRelays((s) => s.workspace)
  const load = useRelays((s) => s.load)
  const openEditor = useRelays((s) => s.openEditor)
  const revoke = useRelays((s) => s.revokeRelay)
  const release = useRelays((s) => s.releaseRelay)
  const columns = useMemo<Column<RelayInfo>[]>(
    () => [
      { key: 'name', header: 'Name', className: 'w-[13%]', cell: (r) => <span className="font-mono break-all">{r.name}</span> },
      { key: 'zone', header: 'Zone', className: 'w-[10%]', cell: (r) => <span className="font-mono">{zoneName(r.zone, zones)}</span> },
      { key: 'address', header: 'Dial address', className: 'w-[20%]', cell: (r) => <DialAddress value={r.dial_address} /> },
      { key: 'fingerprint', header: 'Host key', cell: (r) => <Fingerprint value={r.host_key_fingerprint} label={`${r.name} host key fingerprint`} /> },
      { key: 'state', header: 'State', className: 'w-[16%]', cell: (r) => <RelayStateCell relay={r} /> },
      { key: 'version', header: 'Version', className: 'w-16', cell: (r) => <span className="tabular-nums">{r.version ?? '—'}</span> },
    ],
    [zones],
  )
  return (
    <section aria-labelledby="relays-title" className={SECTION}>
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <h2 id="relays-title" className={H2}>
          Relays
        </h2>
        {canEdit && (
          <Button
            size="sm"
            disabled={zones.length === 0}
            title={zones.length === 0 ? 'Create a zone first.' : undefined}
            onClick={() => {
              openEditor(null)
            }}
          >
            <PlusIcon aria-hidden />
            Register relay
          </Button>
        )}
      </div>
      <SectionIntro guide={GUIDE}>Keel dials each relay and carries its zone&apos;s runner sessions through it. Revoking one cuts it off at once.</SectionIntro>
      {relays.refusal && workspace ? (
        <LoadFailed
          what="relays"
          refusal={relays.refusal}
          onRetry={() => {
            load(workspace)
          }}
        />
      ) : (
        <DataTable
          label="Relays"
          columns={columns}
          rows={relays.value ?? []}
          rowKey={(r) => r.id}
          rowData={(r) => ({ 'data-relay': r.name, 'data-relay-state': r.state })}
          actions={(r) =>
            canEdit &&
            r.state !== 'revoked' && (
              <span className="flex flex-wrap justify-end gap-1">
                {r.state === 'quarantined' && (
                  <Button
                    variant="outline"
                    size="xs"
                    aria-label={`Release relay ${r.name}`}
                    onClick={() => {
                      release(r)
                    }}
                  >
                    <ArrowCounterClockwiseIcon aria-hidden />
                    Release
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="xs"
                  aria-label={`Edit relay ${r.name}`}
                  onClick={() => {
                    openEditor(r)
                  }}
                >
                  <PencilSimpleIcon aria-hidden />
                  Edit
                </Button>
                <Button
                  variant="ghost"
                  size="xs"
                  className="text-status-err"
                  aria-label={`Revoke relay ${r.name}`}
                  onClick={() => {
                    revoke(r)
                  }}
                >
                  <ProhibitIcon aria-hidden />
                  Revoke
                </Button>
              </span>
            )
          }
          empty={
            relays.value === null ? (
              <LoadState loading label="Loading relays…" />
            ) : (
              <span className="text-muted-foreground">
                {zones.length === 0 ? 'No relays. Create a zone, then register its relay.' : 'No relays yet. Hosts in a zone have no path until it has one.'}
              </span>
            )
          }
        />
      )}
      <RelayDialog />
    </section>
  )
}

export default function RelaysTab() {
  const me = useSession((s) => s.me)
  const workspace = useCurrentWorkspace()?.id ?? null
  const load = useRelays((s) => s.load)
  const hostsWorkspace = useHosts((s) => s.workspace)
  const loadHosts = useHosts((s) => s.load)
  useEffect(() => {
    if (workspace !== null) load(workspace)
  }, [workspace, load])
  // The Register dialog offers the workspace's hosts as "runs on host".
  useEffect(() => {
    if (workspace !== null && hostsWorkspace !== workspace) loadHosts(workspace)
  }, [workspace, hostsWorkspace, loadHosts])
  if (workspace === null) return <LoadState loading label="Loading…" />
  const canEdit = canAdminister(me, workspace)
  return (
    <div data-settings-relays className="flex min-w-0 flex-col gap-6">
      <LinkKey />
      <Zones canEdit={canEdit} />
      <Relays canEdit={canEdit} />
    </div>
  )
}
