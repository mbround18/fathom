/**
 * Pure views over a workspace's catalog (spec 066 T051, contracts/ui.md "Repository page"): what
 * the tabs, the Add host dialog and the host detail derive from `CatalogView` without asking the
 * server again.
 */
import type { CatalogGroup, CatalogInventory, CatalogView, HostDecrypt, JobInfo } from '@/protocol/wire'

export type DecryptState = 'ok' | 'cannot' | 'unknown'

/**
 * Whether `host` can decrypt the sops file at `path`: `unknown` until its runner reports an age
 * key, then `ok` when that key is one of the file's recipients, else `cannot`.
 */
export function decryptState(host: HostDecrypt, path: string): DecryptState {
  if (host.age_recipient === null) return 'unknown'
  return host.decrypts.includes(path) ? 'ok' : 'cannot'
}

/** A group in an inventory's tree, with how deep it sits under the roots. */
export interface TreeGroup {
  group: CatalogGroup
  depth: number
}

/**
 * The inventory's groups as a tree, depth first from the groups with no parent, children in name
 * order. A group that is never reached from a root (a cycle) is listed at the end at depth 0, so
 * nothing the catalog holds is hidden.
 */
export function groupTree(inventory: CatalogInventory): TreeGroup[] {
  const byName = new Map(inventory.groups.map((g) => [g.name, g]))
  const out: TreeGroup[] = []
  const seen = new Set<string>()
  const walk = (g: CatalogGroup, depth: number): void => {
    if (seen.has(g.name)) return
    seen.add(g.name)
    out.push({ group: g, depth })
    for (const name of [...g.children].sort()) {
      const child = byName.get(name)
      if (child) walk(child, depth + 1)
    }
  }
  const sorted = [...inventory.groups].sort((a, b) => a.name.localeCompare(b.name))
  for (const g of sorted) if (g.parent === null) walk(g, 0)
  for (const g of sorted) walk(g, 0)
  return out
}

/** Groups a host can be added to: those of the catalog's YAML inventories, `all` aside. */
export function yamlGroups(catalog: CatalogView): string[] {
  const names = catalog.inventories.filter((i) => i.kind === 'yaml').flatMap((i) => i.groups.map((g) => g.name))
  return [...new Set(names)].filter((n) => n !== 'all').sort()
}

/** Profiles worth offering for a new host: `base`, the declared jobs' and the hosts' own. */
export function profileOptions(jobs: readonly Pick<JobInfo, 'profiles'>[], hostProfiles: readonly string[]): string[] {
  return [...new Set(['base', ...jobs.flatMap((j) => j.profiles), ...hostProfiles])].sort()
}

/** Whether any `.sops.yaml` rule already names `key` as a recipient. */
export function keyInRules(catalog: CatalogView, key: string): boolean {
  return catalog.secrets.rules.some((r) => r.age.includes(key))
}

/** The command that re-encrypts a file's data key for its rule's recipients. */
export function updatekeysCommand(path: string): string {
  return `sops updatekeys ${path}`
}

/** A group's address in the Inventory tab: the inventory's path and the group name. */
export function groupItem(inventory: string, group: string): string {
  return `${inventory}#${group}`
}

/** The inverse of `groupItem`; `null` for anything else. */
export function parseGroupItem(item: string): { inventory: string; group: string } | null {
  const at = item.lastIndexOf('#')
  if (at <= 0 || at === item.length - 1) return null
  return { inventory: item.slice(0, at), group: item.slice(at + 1) }
}

/** The stem a job drafted from `playbook` is named after. */
export function playbookStem(playbook: string): string {
  return playbook.replace(/^.*\//, '').replace(/\.ya?ml$/, '')
}
