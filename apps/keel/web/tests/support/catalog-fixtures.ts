/**
 * Spec 066's fixture catalog (T050), modelled on crates/keel-core/tests/fixtures/ansible-repo, with
 * nothing but data in it so unit tests can import it too. Its age keys are synthetic and there is
 * no ciphertext anywhere: the catalog only ever carries key names. ./fake-catalog.ts serves it.
 */
import type { CatalogView, HostDecrypt, JobInfo } from '../../src/protocol/wire'
import { HEAD, NOW } from './fake-repos'

/** Synthetic age public keys (the fixture's), never real ones. */
export const AGE = {
  keel: 'age1psaqfhh8a5h5ekl9ulhkkrwa4430q7mupsaqfhh8a5h5ekl9ulhkkrwa44',
  operator: 'age1ke8u5lr77phxcacrqnh0c8s9du5f68vuke8u5lr77phxcacrqnh0c8s9du',
  kube: 'age18p77qrqh4rs2aumf3t3mc8lck3j7p9098p77qrqh4rs2aumf3t3mc8lck3',
  /** A host's key that no rule names yet. */
  alpha: 'age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq',
} as const

export const INVENTORY = 'inventory/hosts.yml'

/** The fixture repository's catalog, as the server builds it at `HEAD`. */
export function fixtureCatalog(binding: string, hosts: HostDecrypt[] = []): CatalogView {
  return {
    binding,
    commit: HEAD,
    built_at: NOW,
    stale: false,
    error: null,
    playbooks: [
      { path: 'playbooks/cleanup.yaml', plays: [{ name: 'Clean up', hosts: 'all' }], roles: ['cleanup'], imports: [], vars_files: [], unresolved: [], jobs: [] },
      { path: 'playbooks/debian.yaml', plays: [{ name: 'Baseline', hosts: 'all' }], roles: ['base'], imports: [], vars_files: [], unresolved: [], jobs: ['debian'] },
      {
        path: 'playbooks/gameserver.yaml',
        plays: [{ name: 'Game stacks', hosts: 'gameservers' }],
        roles: ['base', 'docker', 'tailscale'],
        imports: ['playbooks/prox-vm.yaml'],
        vars_files: [],
        unresolved: [],
        jobs: ['gameserver'],
      },
      {
        path: 'playbooks/k3s-master.yaml',
        plays: [{ name: 'k3s server', hosts: 'k3s_servers' }],
        roles: ['base', 'k3s_node', 'sops'],
        imports: [],
        vars_files: ['secrets/k3s_servers.sops.yaml'],
        unresolved: [],
        jobs: ['k3s-master'],
      },
      {
        path: 'playbooks/keel-server.yaml',
        plays: [{ name: "Keel's own server", hosts: 'keel_servers' }],
        roles: ['base', 'docker', 'keel_server', 'sops'],
        imports: [],
        vars_files: ['secrets/keel_servers.sops.yaml'],
        unresolved: [],
        jobs: ['keel-server'],
      },
      {
        path: 'playbooks/prox-vm.yaml',
        plays: [{ name: 'VM running a compose stack', hosts: 'compose_hosts' }],
        roles: ['base', 'docker', 'tailscale'],
        imports: [],
        vars_files: [],
        unresolved: [],
        jobs: ['prox-vm'],
      },
      {
        path: 'playbooks/upgrade.yaml',
        plays: [{ name: 'Upgrade packages', hosts: '{{ target | default("all") }}' }],
        roles: [],
        imports: [],
        vars_files: [],
        unresolved: ['hosts: {{ target | default("all") }} is templated'],
        jobs: ['upgrade'],
      },
    ],
    roles: [
      { name: 'base', path: 'roles/base', description: 'The baseline every box gets', defaults: ['base_user', 'base_packages'], dependencies: [], reached_by: ['playbooks/debian.yaml', 'playbooks/gameserver.yaml', 'playbooks/k3s-master.yaml', 'playbooks/keel-server.yaml', 'playbooks/prox-vm.yaml'], external: false },
      { name: 'cleanup', path: 'roles/cleanup', description: null, defaults: [], dependencies: [], reached_by: ['playbooks/cleanup.yaml'], external: false },
      { name: 'docker', path: 'roles/docker', description: 'Docker engine and compose', defaults: ['docker_user'], dependencies: ['base'], reached_by: ['playbooks/gameserver.yaml', 'playbooks/keel-server.yaml', 'playbooks/prox-vm.yaml'], external: false },
      { name: 'k3s_node', path: 'roles/k3s_node', description: null, defaults: ['k3s_version'], dependencies: [], reached_by: ['playbooks/k3s-master.yaml'], external: false },
      { name: 'keel_server', path: 'roles/keel_server', description: null, defaults: ['keel_server_image'], dependencies: ['docker'], reached_by: ['playbooks/keel-server.yaml'], external: false },
      { name: 'sops', path: 'roles/sops', description: null, defaults: ['sops_version'], dependencies: [], reached_by: ['playbooks/k3s-master.yaml', 'playbooks/keel-server.yaml'], external: false },
      { name: 'tailscale', path: 'roles/tailscale', description: null, defaults: ['tailscale_enabled'], dependencies: [], reached_by: ['playbooks/gameserver.yaml', 'playbooks/prox-vm.yaml'], external: false },
      { name: 'community.general.timezone', path: '', description: null, defaults: [], dependencies: [], reached_by: ['playbooks/debian.yaml'], external: true },
    ],
    inventories: [
      {
        path: INVENTORY,
        kind: 'yaml',
        groups: [
          { name: 'all', parent: null, hosts: [], children: ['compose_hosts', 'k3s_servers', 'keel_servers'], vars_file: 'inventory/group_vars/all.yml' },
          { name: 'compose_hosts', parent: 'all', hosts: ['alpha', 'bravo'], children: ['gameservers'], vars_file: 'inventory/group_vars/compose_hosts.yml' },
          { name: 'gameservers', parent: 'compose_hosts', hosts: ['steam'], children: [], vars_file: 'inventory/group_vars/gameservers.yml' },
          { name: 'k3s_servers', parent: 'all', hosts: ['kube1'], children: [], vars_file: 'inventory/group_vars/k3s_servers.yml' },
          { name: 'keel_servers', parent: 'all', hosts: ['keel'], children: [], vars_file: 'inventory/group_vars/keel_servers.yml' },
        ],
      },
    ],
    secrets: {
      rules_file: '.sops.yaml',
      rules: [
        { index: 0, path_regex: '^secrets/keel_servers\\.sops\\.ya?ml$', age: [AGE.keel, AGE.operator], pgp: [] },
        { index: 1, path_regex: '^secrets/.*\\.sops\\.ya?ml$', age: [AGE.keel, AGE.kube], pgp: [] },
      ],
      files: [
        { path: 'secrets/broken.sops.yaml', rule: 1, age: [], pgp: [], lastmodified: null, keys: [], drift: null, problem: 'No sops metadata: the file is not encrypted.', read_by: [] },
        { path: 'secrets/k3s_servers.sops.yaml', rule: 1, age: [AGE.keel], pgp: [], lastmodified: '2026-10-20T09:30:00Z', keys: ['k3s_token', 'k3s_agent_token'], drift: { missing: [AGE.kube], extra: [] }, problem: null, read_by: ['k3s-master'] },
        { path: 'secrets/keel_servers.sops.yaml', rule: 0, age: [AGE.keel, AGE.operator], pgp: [], lastmodified: '2026-10-25T14:00:00Z', keys: ['keel_db_password', 'keycloak_client_secret'], drift: null, problem: null, read_by: ['keel-server'] },
      ],
    },
    problems: [{ path: 'playbooks/upgrade.yaml', line: 3, message: 'hosts: is templated, so its groups are unknown.' }],
    skipped: { files: 0, bytes: 0, reason: null },
    hosts,
  }
}

/** Jobs the fixture's `.keel/jobs.yaml` declares, as `ListJobs` answers them. */
export function fixtureJobs(binding: string): JobInfo[] {
  const job = (name: string, playbook: string, profiles: string[], paths: string[] | null): JobInfo => ({
    id: `job-${name}`,
    binding,
    name,
    playbook,
    profiles,
    inventory: 'fleet',
    limit: null,
    paths,
    credentials: [],
    blocked: null,
    defined_at_commit: HEAD,
  })
  const base = ['ansible.cfg', 'inventory/group_vars/all.yml', 'requirements.yml', 'roles/base/**']
  return [
    job('debian', 'playbooks/debian.yaml', ['debian'], [...base, 'playbooks/debian.yaml']),
    job('gameserver', 'playbooks/gameserver.yaml', ['gameserver'], [...base, 'playbooks/gameserver.yaml', 'roles/docker/**']),
    job('k3s-master', 'playbooks/k3s-master.yaml', ['k3s-master'], [...base, 'playbooks/k3s-master.yaml', 'secrets/k3s_servers.sops.yaml']),
    job('keel-server', 'playbooks/keel-server.yaml', ['keel-server'], [...base, 'playbooks/keel-server.yaml', 'secrets/keel_servers.sops.yaml']),
    job('prox-vm', 'playbooks/prox-vm.yaml', ['prox-vm'], [...base, 'playbooks/prox-vm.yaml', 'roles/docker/**']),
    job('upgrade', 'playbooks/upgrade.yaml', ['debian'], null),
  ]
}

/** The draft keel-core would write for `playbook` (R6), shortened to what the fake knows. */
export function draftYaml(playbook: string): { yaml: string; notes: string[] } {
  const stem = playbook.replace(/^.*\//, '').replace(/\.ya?ml$/, '')
  return {
    yaml: [
      `${stem}:`,
      `  playbook: ${playbook}`,
      `  # No manifest profile is named "${stem}": pick one.`,
      '  profiles: []',
      '  inventory: fleet',
      '  paths:',
      '    - ansible.cfg',
      '    - inventory/group_vars/all.yml',
      `    - ${playbook}`,
      '    - requirements.yml',
      `    - roles/${stem}/**`,
      '  timeout: 30m',
      '',
    ].join('\n'),
    notes: [`No manifest profile is named "${stem}".`],
  }
}

/** A minimal unified diff adding `host` under `group` in the fixture inventory. */
export function inventoryDiff(group: string, host: string): string {
  return [
    `--- a/${INVENTORY}`,
    `+++ b/${INVENTORY}`,
    '@@ -14,3 +14,4 @@',
    `     ${group}:`,
    '       hosts:',
    `+        ${host}:`,
    '',
  ].join('\n')
}
