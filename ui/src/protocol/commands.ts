/**
 * Command builders and section keys shared by the store, `useBackend`, and the fake transport.
 *
 * Section keys mirror `section_keys` in `src/protocol/v1/route.rs` (SectionKey is an opaque
 * string on the wire, so ts-rs cannot export the values).
 */
import type { BackendCmd, SectionKey } from './types'

export const SECTION = {
  system: 'system',
  events: 'events',
  namespaces: 'namespaces',
  pods: 'workloads/pods',
  deployments: 'workloads/deployments',
  customResources: 'custom-resources',
  nodes: 'nodes',
  applications: 'applications',
  services: 'network/services',
  portForwards: 'network/portforwards',
  /** 014-teams-organizations: local §10 Cluster Settings + org-level sync affordances. */
  clusterSettings: 'integrations/cluster-settings',
  /** 014-teams-organizations: shared, non-secret connection profiles + team-store status. */
  teamProfiles: 'team/profiles',
} as const satisfies Record<string, SectionKey>

/** Commands sent once the event subscription is up (mirrors egui's startup in src/app/mod.rs). */
export const BOOTSTRAP_CMDS: readonly BackendCmd[] = [
  { type: 'LoadUiConfig' },
  { type: 'RefreshContexts' },
  // Connects to the current context, then emits ContextChanged, namespaces, pods, deployments.
  { type: 'Initialize' },
]

const REFRESH: Partial<Record<SectionKey, BackendCmd>> = {
  [SECTION.pods]: { type: 'RefreshPods' },
  [SECTION.deployments]: { type: 'RefreshDeployments' },
  [SECTION.events]: { type: 'RefreshEvents' },
  [SECTION.namespaces]: { type: 'RefreshNamespaces' },
  // US3: the CRD catalog itself; instance lists/reads are sent directly by
  // CustomResourcesSection (they need a group/kind/version the section chooses).
  [SECTION.customResources]: { type: 'RefreshCrds' },
  [SECTION.nodes]: { type: 'RefreshNodes' },
  [SECTION.applications]: { type: 'RefreshApplications' },
  [SECTION.services]: { type: 'RefreshServices' },
  [SECTION.portForwards]: { type: 'RefreshPortForwards' },
}

/** The refresh command for a list section, if the UI knows one yet. */
export function refreshCmd(section: SectionKey): BackendCmd | undefined {
  return REFRESH[section]
}

/**
 * The command that reads one object's details. The reply is a `ResourceRead` event whose
 * payload carries `raw_json`, which is also the source for the YAML view (there is no separate
 * YAML command in protocol v1).
 */
export function selectCmd(kind: string, namespace: string, name: string): BackendCmd | undefined {
  switch (kind) {
    case 'Pod':
      return { type: 'SelectPod', data: { namespace, name } }
    case 'Deployment':
      return { type: 'SelectDeployment', data: { namespace, name } }
    case 'Node':
      return { type: 'SelectNode', data: { name } }
    case 'Service':
      return { type: 'SelectService', data: { namespace, name } }
    default:
      return undefined
  }
}

/** `null` means all namespaces, which the backend encodes as an empty filter. */
export function namespaceFilterCmd(ns: string | null): BackendCmd {
  return { type: 'SetNamespaceFilter', data: { namespaces: ns === null ? [] : [ns] } }
}
