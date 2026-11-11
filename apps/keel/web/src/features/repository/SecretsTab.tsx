/**
 * Repository → Secrets (spec 066 T051, FR-014..FR-016): the `.sops.yaml` rules and every sops file
 * with its recipients, whether they drifted from the rule, the jobs that read it and which hosts'
 * age keys can decrypt it. Key names only: Keel never decrypts and never shows a value.
 */
import { ChipList, DataTable, LABEL, ShortCopy, type Column } from '@companion/ui/kit'
import { useMemo } from 'react'

import type { CatalogView, HostDecrypt, SopsFile } from '@/protocol/wire'

import { decryptState, updatekeysCommand, type DecryptState } from './catalog'
import { useSelected } from './address'
import { DetailPane, Flag, ItemButton, ItemLink, Lines, NothingFound, Part, Split } from './parts'

const DECRYPT: Record<DecryptState, { tone: 'ok' | 'err' | 'muted'; text: string }> = {
  ok: { tone: 'ok', text: 'can decrypt' },
  cannot: { tone: 'err', text: 'cannot decrypt' },
  unknown: { tone: 'muted', text: 'no key reported' },
}

function Decrypts({ hosts, path }: { hosts: readonly HostDecrypt[]; path: string }) {
  if (hosts.length === 0) return <span className="text-muted-foreground">No hosts</span>
  return (
    <ul aria-label={`Hosts that can decrypt ${path}`} className="flex min-w-0 flex-wrap gap-1">
      {hosts.map((h) => {
        const state = decryptState(h, path)
        return (
          <li key={h.host} data-decrypt={state} data-host={h.inventory_name}>
            <Flag tone={DECRYPT[state].tone} title={`${h.inventory_name}: ${DECRYPT[state].text}`}>
              <span className="font-mono">{h.inventory_name}</span>
              <span className="sr-only">: {DECRYPT[state].text}</span>
            </Flag>
          </li>
        )
      })}
    </ul>
  )
}

function Detail({ file, catalog }: { file: SopsFile; catalog: CatalogView }) {
  const rule = file.rule === null ? undefined : catalog.secrets.rules.find((r) => r.index === file.rule)
  return (
    <DetailPane
      label="Secrets file"
      title={file.path}
      badges={
        <>
          {file.drift && <Flag data={{ 'data-drift': 'true' }}>recipients out of date</Flag>}
          {file.problem && <Flag tone="err">unreadable</Flag>}
        </>
      }
    >
      {file.problem && (
        <p role="alert" className="break-words text-status-err">
          {file.problem}
        </p>
      )}
      <Part title="Rule">
        {rule ? <code className="font-mono break-all">{rule.path_regex}</code> : <span className="text-muted-foreground">No rule covers it.</span>}
      </Part>
      <Part title="Recipients">
        <Lines label="Recipients" items={[...file.age, ...file.pgp]} />
      </Part>
      {file.drift && (
        <Part title="Out of date">
          <Lines label="Missing recipients" items={file.drift.missing} none="None missing" render={(k) => <>+ {k}</>} />
          <Lines label="Extra recipients" items={file.drift.extra} none="None extra" render={(k) => <>− {k}</>} />
          <p className="text-muted-foreground">Re-encrypt it for the rule&apos;s recipients, then commit:</p>
          <ShortCopy label="command" value={updatekeysCommand(file.path)} short={updatekeysCommand(file.path)} />
        </Part>
      )}
      <Part title="Keys">
        <Lines label="Keys" items={file.keys} />
      </Part>
      <Part title="Read by">
        <Lines label="Read by" items={file.read_by} none="No job reads it." render={(j) => <ItemLink tab="jobs" item={j} />} />
      </Part>
      <Part title="Hosts">
        <Decrypts hosts={catalog.hosts} path={file.path} />
      </Part>
    </DetailPane>
  )
}

export default function SecretsTab({ catalog }: { catalog: CatalogView }) {
  const selected = useSelected()
  const { secrets, hosts } = catalog
  const columns = useMemo<Column<SopsFile>[]>(
    () => [
      {
        key: 'path',
        header: 'File',
        className: 'w-[34%]',
        cell: (f) => (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            <ItemButton item={f.path}>{f.path}</ItemButton>
            {f.drift && <Flag data={{ 'data-drift': 'true' }}>recipients out of date</Flag>}
            {f.problem && <Flag tone="err">unreadable</Flag>}
          </span>
        ),
      },
      { key: 'recipients', header: 'Recipients', cell: (f) => <span className="tabular-nums">{f.age.length + f.pgp.length}</span> },
      { key: 'read', header: 'Read by', cell: (f) => <ChipList label={`Jobs that read ${f.path}`} items={f.read_by} empty="—" /> },
      { key: 'hosts', header: 'Hosts', cell: (f) => <Decrypts hosts={hosts} path={f.path} /> },
    ],
    [hosts],
  )
  const open = secrets.files.find((f) => f.path === selected)
  if (secrets.files.length === 0 && secrets.rules.length === 0) {
    return <NothingFound what="secrets" reason="Keel lists sops-encrypted files and the .sops.yaml rules that cover them." />
  }
  return (
    <Split>
      <div className="flex min-w-0 flex-col gap-3">
        <section aria-label="Creation rules" className="flex min-w-0 flex-col gap-1 text-xs">
          <h2 className={LABEL}>Creation rules{secrets.rules_file ? ` in ${secrets.rules_file}` : ''}</h2>
          {secrets.rules.length === 0 ? (
            <p className="text-muted-foreground">No .sops.yaml rules.</p>
          ) : (
            <ul className="flex min-w-0 flex-col gap-0.5">
              {secrets.rules.map((r) => (
                <li key={r.index} data-sops-rule={r.index} className="min-w-0 break-all">
                  <code className="font-mono">{r.path_regex}</code>{' '}
                  <span className="text-muted-foreground">
                    {r.age.length + r.pgp.length} recipient{r.age.length + r.pgp.length === 1 ? '' : 's'}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
        <DataTable
          label="Secrets files"
          columns={columns}
          rows={secrets.files}
          rowKey={(f) => f.path}
          rowData={(f) => ({ 'data-testid': 'catalog-item', 'data-item': f.path })}
          selected={(f) => f.path === selected}
          empty={<p className="p-3 text-xs text-muted-foreground">No sops-encrypted files.</p>}
        />
      </div>
      {open && <Detail file={open} catalog={catalog} />}
    </Split>
  )
}
