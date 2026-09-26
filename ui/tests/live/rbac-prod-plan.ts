/**
 * The one RBAC plan the opt-in prod specs (`rbac-prod-scoped-*.spec.ts`) may carry out, shared
 * by `global-setup-prod-rbac.ts` (which turns it into the bridge's `--allow-create` list) and the
 * specs. Canonical manifests: `k8s/prod-scoped-agent/rbac.yaml`.
 *
 * Modes (exactly one must be set, see playwright.prod-rbac.config.ts):
 *   FATHOM_PROD_RBAC_SETUP=1   operator identity; bridge may ONLY create the four bindings below.
 *   FATHOM_PROD_RBAC_VERIFY=1  the agent's own scoped kubeconfig; bridge is fully read-only.
 */
import { execFileSync } from 'node:child_process'

import { liveContext } from './env'

export const MANAGED_BY: readonly [string, string] = [
  'app.kubernetes.io/managed-by',
  'fathom-rbac-setup',
]

export const AGENT = { kind: 'ServiceAccount', name: 'fathom-agent', namespace: 'fathom-e2e' }
export const AGENT_USER = `system:serviceaccount:${AGENT.namespace}:${AGENT.name}`
/** The only namespaces the agent may write in. */
export const AGENT_NAMESPACES = ['fathom-dev', 'fathom-e2e'] as const

export const CLUSTER_ROLE = 'fathom-agent-cluster-read'

export interface PlannedBinding {
  kind: 'RoleBinding' | 'ClusterRoleBinding'
  name: string
  namespace: string | null
  /** Always a ClusterRole. */
  roleRef: string
}

export const BINDINGS: readonly PlannedBinding[] = [
  { kind: 'ClusterRoleBinding', name: 'fathom-agent-view', namespace: null, roleRef: 'view' },
  {
    kind: 'ClusterRoleBinding',
    name: 'fathom-agent-cluster-read',
    namespace: null,
    roleRef: CLUSTER_ROLE,
  },
  { kind: 'RoleBinding', name: 'fathom-agent-admin', namespace: 'fathom-dev', roleRef: 'admin' },
  { kind: 'RoleBinding', name: 'fathom-agent-admin', namespace: 'fathom-e2e', roleRef: 'admin' },
]

/** `Kind/name` or `Kind/namespace/name`: the devbridge `--allow-create` syntax. */
export function allowCreateSpec(b: PlannedBinding): string {
  return b.namespace ? `${b.kind}/${b.namespace}/${b.name}` : `${b.kind}/${b.name}`
}

export type ProdRbacMode = 'setup' | 'verify'

export function prodRbacMode(): ProdRbacMode | undefined {
  const setup = process.env.FATHOM_PROD_RBAC_SETUP === '1'
  const verify = process.env.FATHOM_PROD_RBAC_VERIFY === '1'
  if (setup && verify) {
    throw new Error('[prod rbac] set FATHOM_PROD_RBAC_SETUP or FATHOM_PROD_RBAC_VERIFY, not both')
  }
  return setup ? 'setup' : verify ? 'verify' : undefined
}

// ---------------------------------------------------------------------------- read-only kubectl

/**
 * READ-ONLY kubectl against `$FATHOM_LIVE_CONTEXT`: only `get` and `auth can-i` are accepted, so
 * nothing in these specs can write to the cluster except through the guarded bridge, and `get`
 * never touches Secrets.
 */
function kubectlRead(args: readonly string[]): string {
  const [verb, sub] = args
  const ok = verb === 'get' || (verb === 'auth' && sub === 'can-i')
  if (!ok) throw new Error(`[prod rbac] refusing non-read kubectl '${args.join(' ')}'`)
  // `auth can-i get secrets` is an access review and reads nothing; only `get` could read one.
  if (verb === 'get' && args.some((a) => /^(secret|secrets)(\/|$|\.)/.test(a))) {
    throw new Error('[prod rbac] refusing to read Secrets')
  }
  return execFileSync('kubectl', ['--context', liveContext(), ...args], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 60_000,
  })
}

export interface KubeRbacObject {
  metadata: { name: string; namespace?: string; labels?: Record<string, string> }
  roleRef?: { apiGroup: string; kind: string; name: string }
  subjects?: { kind: string; name: string; namespace?: string }[]
  rules?: { apiGroups?: string[]; resources?: string[]; verbs?: string[] }[]
}

/** `kubectl get <kind> <name> [-n ns] -o json`, or `null` if it does not exist. */
export function getRbac(
  kind: 'ClusterRole' | PlannedBinding['kind'],
  name: string,
  namespace: string | null,
): KubeRbacObject | null {
  const resource = `${kind.toLowerCase()}s.rbac.authorization.k8s.io`
  try {
    const out = kubectlRead([
      'get',
      resource,
      name,
      ...(namespace ? ['-n', namespace] : []),
      '-o',
      'json',
    ])
    return JSON.parse(out) as KubeRbacObject
  } catch (err) {
    const stderr = (err as { stderr?: string }).stderr ?? ''
    if (/NotFound|not found/.test(stderr)) return null
    throw err
  }
}

/** Problems with an existing binding vs. the plan (empty = it matches). */
export function bindingMismatches(b: PlannedBinding, obj: KubeRbacObject): string[] {
  const out: string[] = []
  if (obj.roleRef?.kind !== 'ClusterRole' || obj.roleRef.name !== b.roleRef) {
    out.push(`roleRef is ${obj.roleRef?.kind ?? '?'}/${obj.roleRef?.name ?? '?'}`)
  }
  const subjects = obj.subjects ?? []
  const onlyAgent =
    subjects.length === 1 &&
    subjects[0]?.kind === AGENT.kind &&
    subjects[0].name === AGENT.name &&
    subjects[0].namespace === AGENT.namespace
  if (!onlyAgent) out.push(`subjects are ${JSON.stringify(subjects)}`)
  if (obj.metadata.labels?.[MANAGED_BY[0]] !== MANAGED_BY[1]) {
    out.push(`missing label ${MANAGED_BY.join('=')}`)
  }
  return out
}

/** `kubectl auth can-i <verb> <resource> [-n ns|-A] [--as agent]` -> yes/no. */
export function canI(
  verb: string,
  resource: string,
  scope: { namespace?: string; allNamespaces?: boolean },
  asAgent: boolean,
): boolean {
  const args = [
    'auth',
    'can-i',
    verb,
    resource,
    ...(scope.namespace ? ['-n', scope.namespace] : []),
    ...(scope.allNamespaces ? ['-A'] : []),
    ...(asAgent ? [`--as=${AGENT_USER}`] : []),
  ]
  try {
    return kubectlRead(args).trim() === 'yes'
  } catch (err) {
    // `can-i` exits 1 for "no".
    const stdout = (err as { stdout?: string }).stdout ?? ''
    if (stdout.trim() === 'no') return false
    throw err
  }
}
