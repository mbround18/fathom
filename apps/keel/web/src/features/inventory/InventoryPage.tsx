/**
 * Inventories (spec 055 US4, T059): the workspace's inventories, one opened with its revision
 * history, a YAML editor for a managed one (checked as it is typed, so problems show before
 * saving), a diff of any two revisions, and restore of an earlier one. A repository's inventory is
 * read-only here: it changes in the repository, and sync makes the revision.
 *
 * A managed inventory's var schema (JSON Schema for each host's vars) is edited beside it, and
 * checked against the newest revision as it is typed, since a schema it fails is refused.
 *
 * Every change is previewed and confirmed. The editor is CodeMirror, loaded in its own chunk.
 */
import { ArrowCounterClockwiseIcon, PlusIcon, XIcon } from '@phosphor-icons/react'
import { Button, Input } from '@companion/ui'
import { lazy, Suspense, useId, useState, type ReactNode, type SyntheticEvent } from 'react'

import { GrantWorkspacePicker } from '@/app/WorkspacePicker'
import type { InventoryProblem } from '@/protocol/wire'
import { ConfirmDialog } from '@/ui/ConfirmDialog'
import { DataTable } from '@/ui/DataTable'
import { ErrorLine as Alert } from '@/ui/ErrorLine'
import { Field as FormField, H2, LABEL, LINK, Select } from '@/ui/form'
import { LoadState } from '@/ui/LoadState'
import { Page } from '@/ui/Page'
import { useReveal } from '@/ui/reveal'
import { PageHeader } from '@/ui/PageHeader'

import { useInventory, type Opened } from './store'

const YamlEditor = lazy(() => import('./YamlEditor'))

type Revision = Opened['history'][number]
type Host = Opened['revision']['hosts'][number]
type Change = NonNullable<ReturnType<typeof useInventory.getState>['diff']>['changes'][number]
type Item = ReturnType<typeof useInventory.getState>['items'][number]

function Editor(props: { value: string; onChange?: (text: string) => void; label: string; readOnly?: boolean }) {
  return (
    <Suspense fallback={<LoadState loading label="Loading the editor…" />}>
      <YamlEditor {...props} />
    </Suspense>
  )
}

function Field({ label, children, id }: { label: string; children: ReactNode; id: string }) {
  return (
    <FormField label={label} htmlFor={id}>
      {children}
    </FormField>
  )
}

function WorkspacePicker() {
  const workspace = useInventory((s) => s.workspace)
  const load = useInventory((s) => s.load)
  return <GrantWorkspacePicker workspace={workspace} load={load} />
}

function ErrorLine() {
  const error = useInventory((s) => s.error)
  const clear = useInventory((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}

function PreviewDialog() {
  const preview = useInventory((s) => s.preview)
  const confirm = useInventory((s) => s.confirm)
  const cancel = useInventory((s) => s.cancel)
  return <ConfirmDialog preview={preview} onCancel={cancel} onConfirm={confirm} />
}

function Problems({ errors, warnings, label }: { errors: InventoryProblem[]; warnings: InventoryProblem[]; label: string }) {
  if (errors.length + warnings.length === 0) return null
  return (
    <ul aria-label={label} data-problems className="flex min-w-0 flex-col gap-0.5 border border-border p-2 text-xs">
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
    <div className="flex min-w-0 flex-col gap-2">
      <Editor value={draft} onChange={change} label="Inventory YAML" />
      <p aria-live="polite" className="text-xs text-muted-foreground" data-check-state={!edited ? 'saved' : checked ? 'checked' : 'checking'}>
        {!edited ? 'No unsaved changes.' : checked ? (canSave ? 'Checked: no errors.' : 'Fix the errors below before saving.') : 'Checking…'}
      </p>
      {problems && <Problems errors={problems.errors} warnings={problems.warnings} label="Problems" />}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={!canSave} onClick={save}>
          Save revision
        </Button>
        <Button size="sm" variant="ghost" disabled={!edited} onClick={() => setDraft(saved)}>
          Discard changes
        </Button>
      </div>
    </div>
  )
}

/** Edits a managed inventory's var schema. Keyed by the stored schema, so a saved one resets it. */
function EditSchema({ opened }: { opened: Opened }) {
  const { inventory } = opened
  const saved = opened.var_schema ?? ''
  const [draft, setDraft] = useState(saved)
  const validateSchema = useInventory((s) => s.validateSchema)
  const checking = useInventory((s) => s.checkingSchema)
  const validation = useInventory((s) => s.schemaValidation)
  const propose = useInventory((s) => s.propose)

  const edited = draft.trim() !== saved.trim()
  const removing = edited && draft.trim() === ''
  const checked = !edited || removing || (checking === draft && validation?.schema === draft)
  const problems = edited && !removing && validation?.schema === draft ? validation : null
  const canSave = edited && checked && (removing || (problems !== null && problems.errors.length === 0))

  const change = (text: string) => {
    setDraft(text)
    if (text.trim() !== saved.trim() && text.trim() !== '') validateSchema(text)
  }
  const save = () => {
    propose({ type: 'PreviewInventorySchema', data: { inventory: inventory.id, var_schema: removing ? null : draft } })
  }

  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Editor value={draft} onChange={change} label="Var schema JSON" />
      <p aria-live="polite" className="text-xs text-muted-foreground" data-schema-check-state={!edited ? 'saved' : checked ? 'checked' : 'checking'}>
        {!edited
          ? saved === ''
            ? 'No var schema: vars are not checked. Write a JSON Schema to check each host\'s vars.'
            : 'The schema has no unsaved changes.'
          : removing
            ? 'Saving removes the schema.'
            : checked
              ? canSave
                ? 'Checked: the newest revision passes this schema.'
                : 'Fix the errors below before saving.'
              : 'Checking…'}
      </p>
      {problems && <Problems errors={problems.errors} warnings={problems.warnings} label="Schema problems" />}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={!canSave} onClick={save}>
          {removing ? 'Remove schema' : 'Save schema'}
        </Button>
        <Button size="sm" variant="ghost" disabled={!edited} onClick={() => setDraft(saved)}>
          Discard schema changes
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
    <section aria-labelledby={`${inventory.id}-revisions`} className="flex min-w-0 flex-col gap-2">
      <h3 id={`${inventory.id}-revisions`} className={LABEL}>
        History
      </h3>
      <DataTable<Revision>
        label="Revisions"
        rows={history}
        rowKey={(r) => String(r.seq)}
        selected={(r) => r.seq === revision.summary.seq}
        columns={[
          {
            key: 'seq',
            header: 'Revision',
            className: 'w-[26%]',
            cell: (r) => (
              <span className="font-mono">
                {r.seq}
                {r.seq === inventory.head_seq && <span className="ml-1 text-muted-foreground">(newest)</span>}
                {r.restored_from !== null && <span className="ml-1 text-muted-foreground">restored from {r.restored_from}</span>}
              </span>
            ),
          },
          { key: 'status', header: 'Status', cell: (r) => <span className={r.status === 'invalid' ? 'text-status-err' : undefined}>{r.status}</span> },
          { key: 'hosts', header: 'Hosts', cell: (r) => <span className="tabular-nums">{r.host_count}</span> },
          { key: 'by', header: 'By', cell: (r) => r.author ?? (r.source_commit ? r.source_commit.slice(0, 12) : '—') },
          { key: 'created', header: 'Created', cell: (r) => <span className="font-mono">{r.created_at}</span> },
        ]}
        actions={(r) => (
          <>
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
                <ArrowCounterClockwiseIcon aria-hidden />
                Restore
              </Button>
            )}
          </>
        )}
        empty="No revisions yet."
      />
    </section>
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
      <Select id={`${id}-${key}`} value={value} onChange={(e) => set(Number(e.target.value))}>
        {seqs.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
      </Select>
    </Field>
  )

  return (
    <section aria-labelledby={`${id}-title`} className="flex min-w-0 flex-col gap-2">
      <h3 id={`${id}-title`} className={LABEL}>
        Compare revisions
      </h3>
      <form aria-label="Compare revisions" onSubmit={submit} className="flex min-w-0 flex-wrap items-end gap-2">
        {pick(from, setFrom, 'From', 'from')}
        {pick(to, setTo, 'To', 'to')}
        <Button size="sm" type="submit" disabled={from === to}>
          Compare
        </Button>
      </form>
      {shown && (
        <DataTable<Change>
          label={`Changes from ${String(shown.from)} to ${String(shown.to)}`}
          rows={shown.changes}
          rowKey={(c) => c.path}
          rowData={(c) => ({ 'data-change': c.kind })}
          columns={[
            { key: 'path', header: 'Path', className: 'w-[34%]', cell: (c) => <span className="font-mono break-all">{c.path}</span> },
            {
              key: 'kind',
              header: 'Change',
              cell: (c) => (
                <span className={c.kind === 'added' ? 'text-status-ok' : c.kind === 'removed' ? 'text-status-err' : 'text-status-warn'}>
                  {c.kind}
                </span>
              ),
            },
            { key: 'before', header: 'Before', cell: (c) => <span className="font-mono break-all">{c.before ?? '—'}</span> },
            { key: 'after', header: 'After', cell: (c) => <span className="font-mono break-all">{c.after ?? '—'}</span> },
          ]}
          empty="The two revisions hold the same content."
        />
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
  const reveal = useReveal<HTMLElement>(inventory.id)

  return (
    <section ref={reveal} aria-label={`Inventory ${inventory.name}`} className="flex min-w-0 scroll-mt-3 flex-col border border-border">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 font-mono text-xs break-all">{inventory.name}</h2>
        <span className="text-xs text-muted-foreground">
          {inventory.source === 'repo' ? `from ${inventory.path ?? 'a repository'}` : 'managed'} · {inventory.status}
          {inventory.has_schema ? ' · vars checked against a schema' : ''}
        </span>
        <div className="ml-auto flex flex-wrap gap-1">
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
            <XIcon aria-hidden />
            Close
          </Button>
        </div>
      </div>

      <div className="flex min-w-0 flex-col gap-4 p-3">
        <h3 className={H2}>
          Revision {seq}
          {head ? ' (newest)' : ''}
        </h3>
        {!head && (
          <p className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
            An earlier revision, read-only.
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

        <h3 className={H2}>Var schema</h3>
        {editable ? (
          <EditSchema key={`${inventory.id}-${opened.var_schema ?? ''}`} opened={opened} />
        ) : opened.var_schema ? (
          <Editor key={`${inventory.id}-schema`} value={opened.var_schema} label="Var schema JSON" readOnly />
        ) : (
          <p className="text-xs text-muted-foreground">No var schema: vars are not checked.</p>
        )}

        <section aria-labelledby={`${inventory.id}-hosts`} className="flex min-w-0 flex-col gap-2">
          <h3 id={`${inventory.id}-hosts`} className={LABEL}>
            Hosts
          </h3>
          <DataTable<Host>
            label="Hosts"
            rows={revision.hosts}
            rowKey={(h) => h.host}
            columns={[
              { key: 'host', header: 'Host', cell: (h) => <span className="font-mono break-all">{h.host}</span> },
              { key: 'groups', header: 'Groups', cell: (h) => h.groups.join(', ') },
            ]}
            empty="This revision lists no hosts."
          />
        </section>


        <History opened={opened} />
        {opened.history.length > 1 && <Compare key={inventory.id} opened={opened} />}
      </div>
    </section>
  )
}

function NewInventory() {
  const workspace = useInventory((s) => s.workspace)
  const propose = useInventory((s) => s.propose)
  const [name, setName] = useState('')
  const [content, setContent] = useState('')
  const [schema, setSchema] = useState('')
  const id = useId()
  const submit = (e: SyntheticEvent) => {
    e.preventDefault()
    if (!workspace) return
    propose({ type: 'PreviewInventoryCreate', data: { workspace, name: name.trim(), content, var_schema: schema.trim() === '' ? null : schema } })
  }
  return (
    <form aria-label="New inventory" onSubmit={submit} className="flex min-w-0 flex-col gap-3 border border-border bg-muted/20 p-3">
      <h2 className={H2}>New managed inventory</h2>
      <Field label="Name" id={`${id}-name`}>
        <Input id={`${id}-name`} value={name} onChange={(e) => setName(e.target.value)} className="h-8 text-xs" required />
      </Field>
      <Editor value={content} onChange={setContent} label="New inventory YAML" />
      <Editor value={schema} onChange={setSchema} label="New inventory var schema JSON (optional)" />
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
    <Page name="inventory" labelledBy="inventory-title">
      <PageHeader
        id="inventory-title"
        title="Inventory"
        description="The hosts and groups each job runs against, with every revision kept and checked."
        actions={
          <Button size="sm" variant={creating ? 'ghost' : 'default'} aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
            {creating ? <XIcon aria-hidden /> : <PlusIcon aria-hidden />}
            {creating ? 'Close the form' : 'New inventory'}
          </Button>
        }
      >
        <WorkspacePicker />
      </PageHeader>
      <ErrorLine />
      {creating && <NewInventory />}

      <DataTable<Item>
        label="Inventories"
        rows={items}
        rowKey={(i) => i.id}
        selected={(i) => opened?.inventory.id === i.id}
        columns={[
          {
            key: 'name',
            header: 'Name',
            className: 'w-[28%]',
            cell: (i) => (
              <button type="button" className={`${LINK} font-mono break-all`} onClick={() => open(i.id)}>
                {i.name}
              </button>
            ),
          },
          { key: 'source', header: 'Source', cell: (i) => i.source },
          { key: 'status', header: 'Status', cell: (i) => i.status },
          {
            key: 'head',
            header: 'Newest revision',
            cell: (i) => (
              <span className={i.head_status === 'invalid' ? 'text-status-err' : undefined}>
                {i.head_seq === null ? '—' : `${String(i.head_seq)} (${i.head_status ?? ''})`}
              </span>
            ),
          },
          { key: 'hosts', header: 'Hosts', cell: (i) => <span className="tabular-nums">{i.host_count}</span> },
        ]}
        empty="No inventory yet. Bind a repository that declares one, or create a managed one."
      />

      {opened && <Detail opened={opened} />}
      <PreviewDialog />
    </Page>
  )
}
