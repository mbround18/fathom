/**
 * Inventories (spec 055 US4, T059): the workspace's inventories, one opened with its revision
 * history, a YAML editor for a managed one (checked as it is typed, so problems show before
 * saving), a diff of any two revisions, and restore of an earlier one. A repository's inventory is
 * read-only here: it changes in the repository, and sync makes the revision.
 *
 * Every change is previewed and confirmed. The editor is CodeMirror, loaded in its own chunk.
 */
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
} from '@companion/ui'
import { lazy, Suspense, useEffect, useId, useState, type ReactNode, type SyntheticEvent } from 'react'

import { useSession } from '@/app/session-store'
import type { InventoryProblem } from '@/protocol/wire'

import { useInventory, type Opened } from './store'

const YamlEditor = lazy(() => import('./YamlEditor'))

const FIELD =
  'h-8 border border-input bg-background px-2 text-xs outline-none focus-visible:border-accent-neon'
const H2 = 'font-display text-xs font-semibold tracking-wider uppercase'

function Editor(props: { value: string; onChange?: (text: string) => void; label: string; readOnly?: boolean }) {
  return (
    <Suspense fallback={<p className="text-xs text-muted-foreground">Loading the editor…</p>}>
      <YamlEditor {...props} />
    </Suspense>
  )
}

function Field({ label, children, id }: { label: string; children: ReactNode; id: string }) {
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        {label}
      </label>
      {children}
    </div>
  )
}

function WorkspacePicker() {
  const grants = useSession((s) => s.me?.grants ?? [])
  const workspace = useInventory((s) => s.workspace)
  const load = useInventory((s) => s.load)
  const id = useId()
  const first = grants[0]?.workspace ?? null
  const unique = [...new Map(grants.map((g) => [g.workspace, g.workspace_slug])).entries()]

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  if (unique.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin grants one from Teams.
      </p>
    )
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="text-[11px] tracking-wider text-muted-foreground uppercase">
        Workspace
      </label>
      <select id={id} className={FIELD} value={workspace ?? first ?? ''} onChange={(e) => load(e.target.value)}>
        {unique.map(([ws, slug]) => (
          <option key={ws} value={ws}>
            {slug}
          </option>
        ))}
      </select>
    </div>
  )
}

function ErrorLine() {
  const error = useInventory((s) => s.error)
  const clear = useInventory((s) => s.clearError)
  if (!error) return null
  return (
    <p role="alert" className="flex items-center justify-between gap-2 border border-status-err/40 px-2 py-1 text-xs text-status-err">
      <span>{error}</span>
      <Button variant="ghost" size="xs" onClick={clear}>
        Dismiss
      </Button>
    </p>
  )
}

function PreviewDialog() {
  const preview = useInventory((s) => s.preview)
  const confirm = useInventory((s) => s.confirm)
  const cancel = useInventory((s) => s.cancel)
  return (
    <Dialog
      open={preview !== null}
      onOpenChange={(open) => {
        if (!open) cancel()
      }}
    >
      {preview && (
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Confirm</DialogTitle>
            <DialogDescription data-preview-summary>{preview.summary}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={cancel}>
              Cancel
            </Button>
            <Button onClick={confirm}>Confirm</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}

function Problems({ errors, warnings, label }: { errors: InventoryProblem[]; warnings: InventoryProblem[]; label: string }) {
  if (errors.length + warnings.length === 0) return null
  return (
    <ul aria-label={label} data-problems className="flex flex-col gap-0.5 text-xs">
      {errors.map((p, i) => (
        <li key={`e${String(i)}`} data-problem-severity="error" className="break-words text-status-err">
          <span className="font-mono">{p.at}</span>: {p.message}
        </li>
      ))}
      {warnings.map((p, i) => (
        <li key={`w${String(i)}`} data-problem-severity="warning" className="break-words text-status-warn">
          <span className="font-mono">{p.at}</span>: {p.message}
        </li>
      ))}
    </ul>
  )
}

/** Edits the newest revision of a managed inventory. Keyed by revision, so a new one resets it. */
function EditHead({ opened }: { opened: Opened }) {
  const { inventory, revision } = opened
  const saved = revision.source ?? ''
  const [draft, setDraft] = useState(saved)
  const validate = useInventory((s) => s.validate)
  const checking = useInventory((s) => s.checking)
  const validation = useInventory((s) => s.validation)
  const propose = useInventory((s) => s.propose)

  const edited = draft !== saved
  const checked = !edited || (checking === draft && validation?.text === draft)
  const problems = !edited ? revision : validation?.text === draft ? validation : null
  const canSave = edited && checked && problems !== null && problems.errors.length === 0

  const change = (text: string) => {
    setDraft(text)
    if (text !== saved) validate(text)
  }
  const save = () => {
    if (inventory.head_seq === null) return
    propose({ type: 'PreviewInventoryEdit', data: { inventory: inventory.id, base_seq: inventory.head_seq, content: draft } })
  }

  return (
    <div className="flex flex-col gap-2">
      <Editor value={draft} onChange={change} label="Inventory YAML" />
      <p aria-live="polite" className="text-xs text-muted-foreground" data-check-state={!edited ? 'saved' : checked ? 'checked' : 'checking'}>
        {!edited ? 'No unsaved changes.' : checked ? (canSave ? 'Checked: no errors.' : 'Fix the errors below before saving.') : 'Checking…'}
      </p>
      {problems && <Problems errors={problems.errors} warnings={problems.warnings} label="Problems" />}
      <div className="flex gap-2">
        <Button size="sm" disabled={!canSave} onClick={save}>
          Save revision
        </Button>
        <Button size="sm" variant="outline" disabled={!edited} onClick={() => setDraft(saved)}>
          Discard changes
        </Button>
      </div>
    </div>
  )
}

function History({ opened }: { opened: Opened }) {
  const { inventory, revision, history } = opened
  const open = useInventory((s) => s.open)
  const propose = useInventory((s) => s.propose)
  const managed = inventory.source === 'managed' && inventory.status === 'active'
  return (
    <table aria-label="Revisions" className="w-full text-left text-xs">
      <thead className="text-muted-foreground">
        <tr>
          <th className="py-1">Revision</th>
          <th>Status</th>
          <th>Hosts</th>
          <th>By</th>
          <th>Created</th>
          <th>
            <span className="sr-only">Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {history.map((r) => (
          <tr key={r.seq} aria-current={r.seq === revision.summary.seq || undefined} className="border-t border-border">
            <td className="py-1 font-mono">
              {r.seq}
              {r.seq === inventory.head_seq && <span className="ml-1 text-muted-foreground">(newest)</span>}
              {r.restored_from !== null && <span className="ml-1 text-muted-foreground">restored from {r.restored_from}</span>}
            </td>
            <td className={r.status === 'invalid' ? 'text-status-err' : undefined}>{r.status}</td>
            <td>{r.host_count}</td>
            <td>{r.author ?? (r.source_commit ? r.source_commit.slice(0, 12) : '—')}</td>
            <td className="font-mono">{r.created_at}</td>
            <td className="flex gap-1 py-1">
              <Button size="xs" variant="ghost" aria-label={`View revision ${String(r.seq)}`} onClick={() => open(inventory.id, r.seq)}>
                View
              </Button>
              {managed && r.seq !== inventory.head_seq && (
                <Button
                  size="xs"
                  variant="outline"
                  aria-label={`Restore revision ${String(r.seq)}`}
                  onClick={() => propose({ type: 'PreviewInventoryRestore', data: { inventory: inventory.id, seq: r.seq } })}
                >
                  Restore
                </Button>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function Compare({ opened }: { opened: Opened }) {
  const { inventory, history } = opened
  const compare = useInventory((s) => s.compare)
  const diff = useInventory((s) => s.diff)
  const id = useId()
  const seqs = history.map((r) => r.seq)
  const [from, setFrom] = useState(seqs[1] ?? seqs[0] ?? 1)
  const [to, setTo] = useState(seqs[0] ?? 1)
  const shown = diff?.inventory === inventory.id ? diff : null

  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    compare(from, to)
  }
  const pick = (value: number, set: (n: number) => void, label: string, key: string) => (
    <Field label={label} id={`${id}-${key}`}>
      <select id={`${id}-${key}`} className={FIELD} value={value} onChange={(e) => set(Number(e.target.value))}>
        {seqs.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
      </select>
    </Field>
  )

  return (
    <section aria-labelledby={`${id}-title`} className="flex flex-col gap-2">
      <h3 id={`${id}-title`} className={H2}>
        Compare revisions
      </h3>
      <form aria-label="Compare revisions" onSubmit={submit} className="flex flex-wrap items-end gap-2">
        {pick(from, setFrom, 'From', 'from')}
        {pick(to, setTo, 'To', 'to')}
        <Button size="sm" type="submit" disabled={from === to}>
          Compare
        </Button>
      </form>
      {shown && (
        <table aria-label={`Changes from ${String(shown.from)} to ${String(shown.to)}`} className="w-full text-left text-xs">
          <thead className="text-muted-foreground">
            <tr>
              <th className="py-1">Path</th>
              <th>Change</th>
              <th>Before</th>
              <th>After</th>
            </tr>
          </thead>
          <tbody>
            {shown.changes.length === 0 && (
              <tr>
                <td colSpan={4} data-pane-state="empty" className="py-1 text-muted-foreground">
                  The two revisions hold the same content.
                </td>
              </tr>
            )}
            {shown.changes.map((c) => (
              <tr key={c.path} data-change={c.kind} className="border-t border-border font-mono">
                <td className="py-1 break-all">{c.path}</td>
                <td
                  className={
                    c.kind === 'added' ? 'text-status-ok' : c.kind === 'removed' ? 'text-status-err' : 'text-status-warn'
                  }
                >
                  {c.kind}
                </td>
                <td className="break-all">{c.before ?? '—'}</td>
                <td className="break-all">{c.after ?? '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  )
}

function Detail({ opened }: { opened: Opened }) {
  const { inventory, revision } = opened
  const propose = useInventory((s) => s.propose)
  const open = useInventory((s) => s.open)
  const seq = revision.summary.seq
  const head = seq === inventory.head_seq
  const editable = inventory.source === 'managed' && inventory.status === 'active' && head

  return (
    <section aria-label={`Inventory ${inventory.name}`} className="flex flex-col gap-3 border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-xs">{inventory.name}</h2>
        <span className="text-xs text-muted-foreground">
          {inventory.source === 'repo' ? `from ${inventory.path ?? 'a repository'}` : 'managed'} · {inventory.status}
          {inventory.has_schema ? ' · vars checked against a schema' : ''}
        </span>
        <div className="ml-auto flex gap-2">
          {inventory.status === 'active' && (
            <Button
              size="xs"
              variant="outline"
              onClick={() => propose({ type: 'PreviewInventoryDeprecate', data: { inventory: inventory.id } })}
            >
              Deprecate
            </Button>
          )}
          <Button size="xs" variant="ghost" onClick={() => open(null)}>
            Close
          </Button>
        </div>
      </div>

      <h3 className={H2}>
        Revision {seq}
        {head ? ' (newest)' : ''}
      </h3>
      {!head && (
        <p className="text-xs text-muted-foreground">
          An earlier revision, read-only.{' '}
          <Button size="xs" variant="ghost" onClick={() => open(inventory.id)}>
            Back to the newest
          </Button>
        </p>
      )}
      {editable ? (
        <EditHead key={`${inventory.id}-${String(seq)}`} opened={opened} />
      ) : (
        <>
          <Editor
            key={`${inventory.id}-${String(seq)}`}
            value={revision.source ?? revision.content}
            label={`Inventory revision ${String(seq)}`}
            readOnly
          />
          <Problems errors={revision.errors} warnings={revision.warnings} label="Problems" />
        </>
      )}

      <table aria-label="Hosts" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Host</th>
            <th>Groups</th>
          </tr>
        </thead>
        <tbody>
          {revision.hosts.map((h) => (
            <tr key={h.host} className="border-t border-border">
              <td className="py-1 font-mono">{h.host}</td>
              <td>{h.groups.join(', ')}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <History opened={opened} />
      {opened.history.length > 1 && <Compare key={inventory.id} opened={opened} />}
    </section>
  )
}

function NewInventory() {
  const workspace = useInventory((s) => s.workspace)
  const propose = useInventory((s) => s.propose)
  const [name, setName] = useState('')
  const [content, setContent] = useState('')
  const id = useId()
  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (!workspace) return
    propose({ type: 'PreviewInventoryCreate', data: { workspace, name: name.trim(), content, var_schema: null } })
  }
  return (
    <form aria-label="New inventory" onSubmit={submit} className="flex flex-col gap-2 border border-border p-3">
      <Field label="Name" id={`${id}-name`}>
        <Input id={`${id}-name`} value={name} onChange={(e) => setName(e.target.value)} className="h-8 text-xs" required />
      </Field>
      <Editor value={content} onChange={setContent} label="New inventory YAML" />
      <div>
        <Button size="sm" type="submit" disabled={!workspace || name.trim() === ''}>
          Preview inventory
        </Button>
      </div>
    </form>
  )
}

export default function InventoryPage() {
  const items = useInventory((s) => s.items)
  const opened = useInventory((s) => s.opened)
  const open = useInventory((s) => s.open)
  const [creating, setCreating] = useState(false)

  return (
    <section data-page="inventory" aria-labelledby="inventory-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="inventory-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Inventory
        </h1>
        <WorkspacePicker />
        <Button size="sm" variant="outline" className="ml-auto" onClick={() => setCreating((c) => !c)}>
          {creating ? 'Close the form' : 'New inventory'}
        </Button>
      </div>
      <ErrorLine />
      {creating && <NewInventory />}

      <table aria-label="Inventories" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Name</th>
            <th>Source</th>
            <th>Status</th>
            <th>Newest revision</th>
            <th>Hosts</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && (
            <tr>
              <td colSpan={5} data-pane-state="empty" className="py-1 text-muted-foreground">
                No inventory yet. Bind a repository that declares one, or create a managed one.
              </td>
            </tr>
          )}
          {items.map((i) => (
            <tr key={i.id} aria-current={opened?.inventory.id === i.id || undefined} className="border-t border-border">
              <td className="py-1">
                <Button size="xs" variant="ghost" className="font-mono" onClick={() => open(i.id)}>
                  {i.name}
                </Button>
              </td>
              <td>{i.source}</td>
              <td>{i.status}</td>
              <td className={i.head_status === 'invalid' ? 'text-status-err' : undefined}>
                {i.head_seq === null ? '—' : `${String(i.head_seq)} (${i.head_status ?? ''})`}
              </td>
              <td>{i.host_count}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {opened && <Detail opened={opened} />}
      <PreviewDialog />
    </section>
  )
}
