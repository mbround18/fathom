/**
 * Repository → Inventory (spec 066 T051): each inventory's groups as a tree with their hosts. A
 * group's detail offers Add host here (T053), which opens Add host with that group chosen. Only a
 * YAML inventory can be edited that way.
 */
import { PlusIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { ChipList, DataTable, H2, type Column } from '@companion/ui/kit'
import { useMemo } from 'react'

import { useSession } from '@/app/session-store'
import { useAddHost } from '@/features/hosts/add-host-store'
import { canOperate } from '@/features/hosts/workspaces'
import type { CatalogGroup, CatalogInventory, CatalogView } from '@/protocol/wire'

import { groupItem, groupTree, parseGroupItem, type TreeGroup } from './catalog'
import { select, useSelected } from './address'
import { DetailPane, Flag, ItemButton, Lines, NothingFound, Part, Split } from './parts'

function Detail({ inventory, group, canAdd }: { inventory: CatalogInventory; group: CatalogGroup; canAdd: boolean }) {
  const show = useAddHost((s) => s.show)
  const editable = inventory.kind === 'yaml' && group.name !== 'all'
  return (
    <DetailPane
      label="Group"
      title={group.name}
      badges={<Flag tone="muted">{inventory.path}</Flag>}
      actions={
        canAdd &&
        editable && (
          <Button
            size="sm"
            variant="outline"
            data-add-host-here
            onClick={() => {
              show(group.name)
            }}
          >
            <PlusIcon aria-hidden />
            Add host here
          </Button>
        )
      }
    >
      {group.parent && (
        <Part title="Parent">
          <ItemButton item={groupItem(inventory.path, group.parent)}>{group.parent}</ItemButton>
        </Part>
      )}
      <Part title="Children">
        <Lines label="Children" items={group.children} render={(c) => <ItemButton item={groupItem(inventory.path, c)}>{c}</ItemButton>} />
      </Part>
      <Part title="Hosts">
        <Lines label="Hosts" items={group.hosts} />
      </Part>
      {group.vars_file && (
        <Part title="Group vars">
          <code className="font-mono break-all">{group.vars_file}</code>
        </Part>
      )}
    </DetailPane>
  )
}

function Inventory({ inventory, selected }: { inventory: CatalogInventory; selected: string | null }) {
  const rows = useMemo(() => groupTree(inventory), [inventory])
  const columns = useMemo<Column<TreeGroup>[]>(
    () => [
      {
        key: 'group',
        header: 'Group',
        className: 'w-[40%]',
        cell: ({ group, depth }) => (
          <span className="flex min-w-0" style={{ paddingLeft: `${String(depth)}rem` }}>
            <ItemButton item={groupItem(inventory.path, group.name)}>{group.name}</ItemButton>
          </span>
        ),
      },
      { key: 'hosts', header: 'Hosts', cell: ({ group }) => <ChipList label={`Hosts in ${group.name}`} items={group.hosts} empty="—" /> },
    ],
    [inventory.path],
  )
  return (
    <section aria-labelledby={`inv-${inventory.path}`} className="flex min-w-0 flex-col gap-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <h2 id={`inv-${inventory.path}`} className={`${H2} font-mono break-all`}>
          {inventory.path}
        </h2>
        <Flag tone="muted">{inventory.kind}</Flag>
      </div>
      {inventory.kind === 'dynamic' ? (
        <p className="text-xs text-muted-foreground">A dynamic inventory: its groups are only known when it runs.</p>
      ) : (
        <DataTable
          label={`Groups in ${inventory.path}`}
          columns={columns}
          rows={rows}
          rowKey={(t) => t.group.name}
          rowData={(t) => ({ 'data-testid': 'catalog-item', 'data-item': groupItem(inventory.path, t.group.name) })}
          selected={(t) => groupItem(inventory.path, t.group.name) === selected}
          empty={<p className="p-3 text-xs text-muted-foreground">No groups.</p>}
        />
      )}
    </section>
  )
}

export default function InventoryTab({ catalog, workspace }: { catalog: CatalogView; workspace: string }) {
  const selected = useSelected()
  const me = useSession((s) => s.me)
  const at = selected === null ? null : parseGroupItem(selected)
  const inventory = at && catalog.inventories.find((i) => i.path === at.inventory)
  const group = at && inventory?.groups.find((g) => g.name === at.group)
  if (catalog.inventories.length === 0) return <NothingFound what="inventories" reason="Keel reads the inventory the manifest names, and inventory/ or hosts files." />
  return (
    <Split>
      <div className="flex min-w-0 flex-col gap-4">
        {catalog.inventories.map((i) => (
          <Inventory key={i.path} inventory={i} selected={selected} />
        ))}
      </div>
      {inventory && group ? (
        <Detail inventory={inventory} group={group} canAdd={canOperate(me, workspace)} />
      ) : (
        selected !== null && (
          <p className="text-xs text-muted-foreground">
            Nothing named {selected} in this catalog.{' '}
            <button
              type="button"
              className="underline"
              onClick={() => {
                select(null)
              }}
            >
              Clear
            </button>
          </p>
        )
      )}
    </Split>
  )
}
