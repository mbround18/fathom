/**
 * Repository → Playbooks (spec 066 T051): every playbook the catalog found, the roles it reaches
 * and the jobs that run it. The detail pane offers Draft job (T052).
 */
import { FileCodeIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { DataTable, type Column } from '@companion/ui/kit'
import { useMemo, useState } from 'react'

import type { CatalogPlaybook, CatalogView } from '@/protocol/wire'

import { DraftJobDialog } from './DraftJobDialog'
import { useSelected } from './address'
import { DetailPane, Flag, ItemButton, ItemLink, Lines, NothingFound, Part, Split } from './parts'

function Detail({ playbook, onDraft }: { playbook: CatalogPlaybook; onDraft: () => void }) {
  return (
    <DetailPane
      label="Playbook"
      title={playbook.path}
      badges={playbook.unresolved.length > 0 && <Flag title="Some references are templated">unresolved</Flag>}
      actions={
        <Button size="sm" variant="outline" data-draft-job onClick={onDraft}>
          <FileCodeIcon aria-hidden />
          Draft job
        </Button>
      }
    >
      <Part title="Plays">
        <ul aria-label="Plays" className="flex min-w-0 flex-col gap-0.5">
          {playbook.plays.map((p, i) => (
            // Plays have no identity but their place in the file.
            <li key={String(i)} className="min-w-0 break-all">
              {p.name ?? <span className="text-muted-foreground">unnamed play</span>} on <code className="font-mono">{p.hosts}</code>
            </li>
          ))}
        </ul>
      </Part>
      <Part title="Roles">
        <Lines label="Roles" items={playbook.roles} render={(r) => <ItemLink tab="roles" item={r} />} />
      </Part>
      {playbook.imports.length > 0 && (
        <Part title="Imports">
          <Lines label="Imports" items={playbook.imports} render={(p) => <ItemLink tab="playbooks" item={p} />} />
        </Part>
      )}
      {playbook.vars_files.length > 0 && (
        <Part title="Vars files">
          <Lines label="Vars files" items={playbook.vars_files} />
        </Part>
      )}
      {playbook.unresolved.length > 0 && (
        <Part title="Not followed">
          <Lines label="Not followed" items={playbook.unresolved} />
        </Part>
      )}
      <Part title="Jobs">
        <Lines label="Jobs" items={playbook.jobs} none="No job runs it yet." render={(j) => <ItemLink tab="jobs" item={j} />} />
      </Part>
    </DetailPane>
  )
}

export default function PlaybooksTab({ catalog, workspace }: { catalog: CatalogView; workspace: string }) {
  const selected = useSelected()
  const [drafting, setDrafting] = useState<string | null>(null)
  const columns = useMemo<Column<CatalogPlaybook>[]>(
    () => [
      {
        key: 'path',
        header: 'Playbook',
        className: 'w-[40%]',
        cell: (p) => (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            <ItemButton item={p.path}>{p.path}</ItemButton>
            {p.unresolved.length > 0 && <Flag>unresolved</Flag>}
          </span>
        ),
      },
      { key: 'roles', header: 'Roles', cell: (p) => <span className="tabular-nums">{p.roles.length}</span> },
      {
        key: 'jobs',
        header: 'Jobs',
        cell: (p) => (p.jobs.length === 0 ? <span className="text-muted-foreground">none</span> : <span className="font-mono break-all">{p.jobs.join(', ')}</span>),
      },
    ],
    [],
  )
  const open = catalog.playbooks.find((p) => p.path === selected)
  if (catalog.playbooks.length === 0) return <NothingFound what="playbooks" reason="Keel looks for playbooks at the top of the repository and under playbooks/." />
  return (
    <Split>
      <DataTable
        label="Playbooks"
        columns={columns}
        rows={catalog.playbooks}
        rowKey={(p) => p.path}
        rowData={(p) => ({ 'data-testid': 'catalog-item', 'data-item': p.path })}
        selected={(p) => p.path === selected}
        empty={null}
      />
      {open && (
        <Detail
          playbook={open}
          onDraft={() => {
            setDrafting(open.path)
          }}
        />
      )}
      <DraftJobDialog
        workspace={workspace}
        playbook={drafting}
        onClose={() => {
          setDrafting(null)
        }}
      />
    </Split>
  )
}
