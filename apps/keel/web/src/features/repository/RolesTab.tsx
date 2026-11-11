/**
 * Repository → Roles (spec 066 T051): each role, its defaults by name (never their values), its
 * dependencies and the playbooks that reach it. A collection role is listed, not followed.
 */
import { DataTable, type Column } from '@companion/ui/kit'
import { useMemo } from 'react'

import type { CatalogRole, CatalogView } from '@/protocol/wire'

import { useSelected } from './address'
import { DetailPane, Flag, ItemButton, ItemLink, Lines, NothingFound, Part, Split } from './parts'

function Detail({ role }: { role: CatalogRole }) {
  return (
    <DetailPane label="Role" title={role.name} badges={role.external && <Flag tone="muted">external</Flag>}>
      {role.description && <p className="break-words">{role.description}</p>}
      {!role.external && (
        <Part title="Path">
          <code className="font-mono break-all">{role.path}</code>
        </Part>
      )}
      <Part title="Defaults">
        <Lines label="Defaults" items={role.defaults} none={role.external ? 'Not read: a collection role.' : 'None'} />
      </Part>
      <Part title="Depends on">
        <Lines label="Depends on" items={role.dependencies} render={(r) => <ItemLink tab="roles" item={r} />} />
      </Part>
      <Part title="Reached by">
        <Lines label="Reached by" items={role.reached_by} none="No playbook reaches it." render={(p) => <ItemLink tab="playbooks" item={p} />} />
      </Part>
    </DetailPane>
  )
}

export default function RolesTab({ catalog }: { catalog: CatalogView }) {
  const selected = useSelected()
  const columns = useMemo<Column<CatalogRole>[]>(
    () => [
      {
        key: 'name',
        header: 'Role',
        className: 'w-[35%]',
        cell: (r) => (
          <span className="flex min-w-0 flex-wrap items-center gap-1">
            <ItemButton item={r.name}>{r.name}</ItemButton>
            {r.external && <Flag tone="muted">external</Flag>}
          </span>
        ),
      },
      { key: 'description', header: 'Description', cell: (r) => r.description ?? <span className="text-muted-foreground">—</span> },
      { key: 'reached', header: 'Playbooks', cell: (r) => <span className="tabular-nums">{r.reached_by.length}</span> },
    ],
    [],
  )
  const open = catalog.roles.find((r) => r.name === selected)
  if (catalog.roles.length === 0) return <NothingFound what="roles" reason="Keel reads roles from roles/ and from what the playbooks name." />
  return (
    <Split>
      <DataTable
        label="Roles"
        columns={columns}
        rows={catalog.roles}
        rowKey={(r) => r.name}
        rowData={(r) => ({ 'data-testid': 'catalog-item', 'data-item': r.name })}
        selected={(r) => r.name === selected}
        empty={null}
      />
      {open && <Detail role={open} />}
    </Split>
  )
}
