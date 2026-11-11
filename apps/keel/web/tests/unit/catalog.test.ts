/**
 * Spec 066 T051: the catalog views the Repository page, the Add host dialog and the host detail
 * derive (src/features/repository/catalog.ts). Pure, against the fake backend's fixture catalog.
 */
import { describe, expect, it } from 'vitest'

import {
  decryptState,
  groupItem,
  groupTree,
  keyInRules,
  parseGroupItem,
  playbookStem,
  profileOptions,
  updatekeysCommand,
  yamlGroups,
} from '../../src/features/repository/catalog'
import { AGE, fixtureCatalog, fixtureJobs, INVENTORY } from '../support/catalog-fixtures'

const catalog = fixtureCatalog('binding-1')

describe('decryptState', () => {
  const host = { host: 'h1', inventory_name: 'keel', age_recipient: AGE.keel, decrypts: ['secrets/keel_servers.sops.yaml'] }

  it('is ok for a file the key is a recipient of', () => {
    expect(decryptState(host, 'secrets/keel_servers.sops.yaml')).toBe('ok')
  })

  it('is cannot for a file it is not', () => {
    expect(decryptState(host, 'secrets/k3s_servers.sops.yaml')).toBe('cannot')
  })

  it('is unknown until the runner reports a key', () => {
    expect(decryptState({ ...host, age_recipient: null, decrypts: [] }, 'secrets/keel_servers.sops.yaml')).toBe('unknown')
  })
})

describe('groupTree', () => {
  it('walks from the roots, children in name order, each under its parent', () => {
    const inventory = catalog.inventories[0]
    if (!inventory) throw new Error('no inventory')
    expect(groupTree(inventory).map((t) => [t.group.name, t.depth])).toEqual([
      ['all', 0],
      ['compose_hosts', 1],
      ['gameservers', 2],
      ['k3s_servers', 1],
      ['keel_servers', 1],
    ])
  })

  it('still lists a group no root reaches', () => {
    const tree = groupTree({
      path: 'x.yml',
      kind: 'yaml',
      groups: [
        { name: 'a', parent: 'b', hosts: [], children: ['b'], vars_file: null },
        { name: 'b', parent: 'a', hosts: [], children: ['a'], vars_file: null },
      ],
    })
    expect(tree.map((t) => t.group.name)).toEqual(['a', 'b'])
  })
})

describe('yamlGroups', () => {
  it('offers every YAML inventory group but all, sorted', () => {
    expect(yamlGroups(catalog)).toEqual(['compose_hosts', 'gameservers', 'k3s_servers', 'keel_servers'])
  })

  it('offers nothing from an INI or dynamic inventory', () => {
    const ini = { ...catalog, inventories: catalog.inventories.map((i) => ({ ...i, kind: 'ini' as const })) }
    expect(yamlGroups(ini)).toEqual([])
  })
})

describe('profileOptions', () => {
  it('is base, the jobs’ and the hosts’ profiles, once each, sorted', () => {
    expect(profileOptions(fixtureJobs('b'), ['web', 'debian'])).toEqual(['base', 'debian', 'gameserver', 'k3s-master', 'keel-server', 'prox-vm', 'web'])
  })
})

describe('keyInRules', () => {
  it('finds a key a rule names, and not one no rule does', () => {
    expect(keyInRules(catalog, AGE.kube)).toBe(true)
    expect(keyInRules(catalog, AGE.alpha)).toBe(false)
  })
})

describe('addresses', () => {
  it('round-trips a group item', () => {
    expect(parseGroupItem(groupItem(INVENTORY, 'k3s_servers'))).toEqual({ inventory: INVENTORY, group: 'k3s_servers' })
  })

  it('refuses anything that is not one', () => {
    expect(parseGroupItem('playbooks/debian.yaml')).toBeNull()
    expect(parseGroupItem('#x')).toBeNull()
    expect(parseGroupItem('x#')).toBeNull()
  })

  it('names a draft after the playbook stem', () => {
    expect(playbookStem('playbooks/k3s-master.yaml')).toBe('k3s-master')
    expect(playbookStem('site.yml')).toBe('site')
  })

  it('writes the updatekeys command', () => {
    expect(updatekeysCommand('secrets/k3s_servers.sops.yaml')).toBe('sops updatekeys secrets/k3s_servers.sops.yaml')
  })
})
