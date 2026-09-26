/**
 * App state (data-model.md §3). One Zustand store; components subscribe with narrow selectors.
 *
 * Backend data arrives only through `ingest(batch)` (./ingest.ts), which applies one `set()`
 * per batch. Actions that need the backend call `send`, which `useBackend` wires to the
 * transport; until then it is a no-op.
 */
import { create } from 'zustand'

import { initialIdentityState, type IdentityState } from '@/features/identity/state'
import { initialTeamsState, type TeamsState } from '@/features/teams/state'
import { initialRolesState, type RolesState } from '@/features/roles/state'
import { resolveProfile } from '@/layout/defaults'
import type { WidthClass } from '@/layout/width-class'
import { namespaceFilterCmd, refreshCmd, SECTION, selectCmd } from '@/protocol/commands'
import type { AiSlice } from '@/features/ai-assist/types'
import type {
  AdoptedConnectionProfile,
  DaemonStatus,
  AgentAuthMode,
  AgentConversation,
  AuditLogEntry,
  BackendCmd,
  ClusterSettings,
  ConnectionProfile,
  ContextConnectionStatus,
  KindClusterSummary,
  KindPrerequisites,
  CrdCatalogEntry,
  DiskUsage,
  ClusterVersionSummary,
  CertManagerOverview,
  CertificateChain,
  FluxOverview,
  GatekeeperOverview,
  VeleroOverview,
  PodVulnerabilityInspection,
  TrivyOverview,
  VulnInspectError,
  KedaOverview,
  KedaResourceDetails,
  HubbleTrafficSummary,
  ImageInspectError,
  ImageInspection,
  KnativeOverview,
  KubeadmDetectionResult,
  KubernetesEvent,
  LayoutProfile,
  MetricsStackScanResult,
  NamespaceAccessResolved,
  NodeShellAccessCheck,
  NodeShellPhase,
  NodeShellSettings,
  OrgClusterSettings,
  OrphanedNodeShellPod,
  RbacAccessResult,
  ResourceListEvent,
  ResourceReadEvent,
  Route,
  SectionKey,
  SlotConfig,
  TeamPublishRejection,
  TeamStoreStatus,
  ThemePrefs,
  Density,
  ToolKind,
  FleetContextSummary,
  CloudAuthInfo,
  PluginError,
  TunnelStatus,
  UiConfig,
  GrafanaSettings,
  PrometheusSettings,
} from '@/protocol/types'

import { descriptorFor } from '@/resources'

import { initialDaemon, type DaemonSlice } from './handlers/daemon'
import { initialFederation, type FederationSlice } from './handlers/federation'
import type { ObjectRef } from '@/resources/types'

import {
  buildApplyCommand,
  discard as discardSession,
  editYaml,
  markApplying,
  markFailed,
  openCreateSession,
  openSession,
  reviewDiff as reviewSessionDiff,
  type ResourceEditSession,
  type ResourceEditTarget,
} from './editSession'
import {
  initialHelmMutationSession,
  startApply as startHelmApply,
  startPreview as startHelmPreview,
  reset as resetHelmMutationSession,
  type HelmMutationSession,
} from './helmMutation'
import { detailKey, listKey, type DetailKey, type ListKey } from './keys'
import { DEFAULT_AUDIT_FILTER, type AuditLogFilter } from './auditLogFilter'
import { RingBuffer } from './ring-buffer'
import { waitForCommandResult } from './actions'
import type { HostedState } from './handlers/hosted'

export type { ResourceEditSession, ResourceEditTarget, EditSessionStatus } from './editSession'
export type { HelmMutationSession, HelmMutationStatus } from './helmMutation'

export { detailKey, listKey, listKeyOf, type DetailKey, type ListKey } from './keys'

// ------------------------------------------------------------------ UI-owned types

export type PaneId = string
export type PaneKind = SlotConfig['kind']

/** Per-pane view state. Lives here, not in components, so re-layout never loses it (FR-007). */
export interface PaneState {
  id: PaneId
  kind: PaneKind
  tool?: ToolKind
  route?: Route
  search: string
  /** List: the row key open in the inspector (inverse video). */
  selection?: string
  scrollTop: number
  /** List: column sort; `undefined` keeps the backend's order. */
  sort?: { id: string; desc: boolean }
  /** List: the keyboard cursor's row key. */
  focusedRow?: string
  /** List: column ids the budget currently shows (the inspector lists the hidden ones, FR-009). */
  visibleColumns?: string[]
  /** Inspector: the object it shows, and the list pane it was opened from. */
  target?: ObjectRef
  source?: PaneId
  /**
   * Inspector: prior `target`s drilled through via related-resource links to reach the current
   * one, oldest first (data-model.md §"Frontend: drill-down / back-stack state", research.md
   * R6). Empty when opened directly from a list row. Session-transient, never persisted.
   */
  history?: ObjectRef[]
  /**
   * Tool (topology): last camera transform (research.md R4), mirroring how `scrollTop` persists
   * a list pane's scroll position across re-layout. `undefined` until the pane's first render.
   */
  topology?: { scale: number; x: number; y: number }
}

/** data-model.md §4: starting → ready → backend-gone (fatal; retry = app restart). */
export type ConnectionStatus = 'starting' | 'ready' | 'backend-gone'

export interface Notice {
  level: 'info' | 'warn' | 'error'
  message: string
  at: number
}

/** Drop the status-bar notice: it described the previous context/namespace/section. */
function withoutNotice(conn: ConnState): ConnState {
  const rest = { ...conn }
  delete rest.lastNotice
  return rest
}

export interface ConnState {
  status: ConnectionStatus
  /** Envelopes the bridge discarded because the UI fell behind (backpressure), cumulative. */
  dropped: number
  /** Why the connection is unusable (backend gone, protocol mismatch). Shown verbatim. */
  fatal?: string
  lastNotice?: Notice
}

/** A context absent from `SessionState.contextStatuses` renders as `NotConnected` by convention
 * (data-model.md) — this is not itself a wire status, just "never probed/reported yet". */
export interface ContextStatusEntry {
  status: ContextConnectionStatus
  /** Present only when `status === 'Error'` (012-incluster-daemon-federation P1). */
  error?: string
  /** 035: typed cloud-auth plugin failure (present when the error came from an exec plugin). */
  pluginError?: PluginError
}

/** Autoscalers Inspector state: the selected ScaledObject/ScaledJob's detail, or a not-found reply. */
export type KedaSelection =
  | { state: 'ready'; details: KedaResourceDetails }
  | { state: 'gone'; kind: string; namespace: string; name: string }

export interface SessionState {
  /** Kubeconfig context names. Empty with `contextsLoaded` means no kubeconfig. */
  contexts: string[]
  contextsLoaded: boolean
  activeContext?: string
  namespaces: string[]
  /** `null` means all namespaces. */
  activeNamespace: string | null
  /** The backend's namespace filter as last reported (`[]` = all). */
  namespaceFilter: string[]
  /** Live per-context connection status (012-incluster-daemon-federation P1, FR-002). Last
   * write per context wins — the backend never double-writes the same context (data-model.md
   * §"Frontend"). */
  contextStatuses: Record<string, ContextStatusEntry>
  /** 035: detected exec-plugin (cloud auth) info per context; refreshed by `CloudAuthDetected`. */
  cloudAuth: Record<string, CloudAuthInfo>
  /** 027-fleet-rollup-dashboard: latest poll summary per context; runtime-only, populated only
   * while the Fleet Dashboard is open. Last write per context wins. */
  fleetSummaries: Record<string, FleetContextSummary>
  /** 034-simultaneous-federation: the live federated set and its rollups (backend-owned). */
  federation: FederationSlice
}

type RowOf<T> = T extends readonly (infer R)[] ? R : T extends { events: (infer E)[] } ? E : never
/** Any row a `ResourceList` event can carry. */
export type ListRow = RowOf<ResourceListEvent['data']>

export interface ListState {
  /** Which `ResourceListEvent` variant filled this list. */
  eventType?: ResourceListEvent['type']
  rows: readonly ListRow[]
  /** For events: the backend's total before truncation. */
  total?: number
  latestRequest?: number
  status: 'idle' | 'loading' | 'ready' | 'error'
  error?: string
  updatedAt: number
}

/**
 * Every `ResourceReadEvent` variant except the two 010-container-image-inspection ones,
 * 013-agent-sdk-ops's `AgentConversationUpdated`, and the three 011-rbac-oidc-administration
 * ones, none of which are stored here — they live in `imageInspections`/`agentConversations`/
 * `rbacAccess` instead (their payloads have no `name` field for this generic namespace/kind/name
 * keying). Must match `handlers/read.ts`'s identical `GenericReadEvent`.
 */
type GenericReadEvent = Exclude<
  ResourceReadEvent,
  | { type: 'ContainerImageInspected' | 'ContainerImageInspectFailed' | 'AgentConversationUpdated' }
  // 021-trivy-vulnerability-pane: keyed by namespace/pod, routed to `handlePodVulnInspect`.
  | { type: 'PodVulnerabilitiesInspected' | 'PodVulnerabilitiesInspectFailed' }
  | {
      type:
        | 'RbacSelfAccessUpdated'
        | 'ResourceMutationAccessChecked'
        | 'NamespaceAccessResolved'
        | 'KubeadmDetectionUpdated'
    }
  // 024-pod-exec-terminal: routed by `handleExecAccess` into `execAccess`.
  | { type: 'PodExecAccessChecked' }
>

/** 024-pod-exec-terminal: result of a `CheckPodExecAccess` pre-check. */
export interface ExecAccessCheckState {
  allowed: boolean
  reason?: string
  /** Bumped on every reply so a repeat check for the same key is observable. */
  rev: number
}

export function execAccessKey(namespace: string, name: string, container: string | null): string {
  return `${namespace}/${name}/${container ?? ''}`
}

export interface DetailState {
  /** Object kind derived from the event (`PodDetailsUpdated` → `Pod`, `PodMetricsUpdated` → `PodMetrics`). */
  kind: string
  eventType: GenericReadEvent['type']
  data: GenericReadEvent['data']
  latestRequest?: number
  updatedAt: number
}

export interface LogStreamState {
  kind: 'logs'
  lines: RingBuffer<string>
  /** Bumped on every append; the buffer itself is mutated in place. */
  rev: number
  follow: boolean
  status: 'open' | 'closed' | 'error'
  reason?: string
}

export interface ExecStreamState {
  kind: 'exec'
  status: 'attaching' | 'active' | 'ended' | 'error'
  reason?: string
  /** Chosen container, for display. */
  container?: string
  /**
   * Decoded output chunks, bounded like `LogStreamState.lines` (constitution Principle V) —
   * xterm.js owns the *rendered* scrollback (research.md R5), this is only a short-lived
   * handoff queue `PodTerminalTab` drains and writes to the terminal as chunks arrive.
   */
  chunks: RingBuffer<string>
  /** Bumped on every appended chunk; the buffer itself is mutated in place. */
  rev: number
}

export interface TunnelState {
  kind: 'portforward'
  status: TunnelStatus
  localEndpoint: string | null
}

/**
 * 009: decorative "which Service, if any, this forward was launched from" info (data-model.md
 * "New frontend-only state"). Ephemeral, matched by namespace/pod/local_port/remote_port right
 * after a Service-launched `StartPortForward` succeeds, against the next `PortForwardsUpdated`
 * reply that contains a matching, not-yet-tagged row. Lost on reload; purely decorative, never
 * the tunnel's identity.
 */
export interface PortForwardOrigin {
  service: string
}

/** A Service-launched forward-start still waiting to be matched to its tunnel id, keyed by
 * `${namespace}|${pod}|${localPort}|${remotePort}` (data-model.md's matching rule). */
export type PendingPortForwardOrigins = Record<string, string>

export function pendingPortForwardOriginKey(
  namespace: string,
  pod: string,
  localPort: number,
  remotePort: number,
): string {
  return `${namespace}|${pod}|${String(localPort)}|${String(remotePort)}`
}

export interface PendingAction {
  /** Shown in the result toast, e.g. "Delete 3 pods". */
  label: string
  /** Pane whose selection is cleared when the action succeeds. */
  paneId?: PaneId
  sentAt: number
}

/** An action awaiting the operator's confirmation (the dialog host renders it). */
export interface ActionPrompt {
  section: SectionKey
  actionId: string
  /** Snapshot of the rows at the time the action was chosen; the dialog names each one. */
  rows: readonly ListRow[]
  /** List pane whose selection the action came from (cleared on success; focus returns here). */
  paneId?: PaneId
  /** Where focus goes when the dialog closes, if it is still in the document. */
  returnFocus?: HTMLElement | null
}

export interface MetricsState {
  diskUsage: DiskUsage[]
  stackScan?: MetricsStackScanResult
}

/** Live progress for an in-flight (or just-finished) `DrainNode` (data-model.md "DrainProgress"). */
export interface DrainProgress {
  totalPods: number
  evicted: string[]
  blockedOn: { pod: string; reason: string }[]
  ignoredDaemonSetPods: string[]
  /** Frontend-only: set optimistically when the operator cancels, before the backend confirms. */
  cancelled: boolean
}

/**
 * One node action in flight, keyed by node name (data-model.md "Node Action"). Ephemeral,
 * frontend-held UI state — not persisted, not sent to the backend beyond the commands
 * `NodeActionsPanel` sends when the operator confirms.
 */
export interface NodeAction {
  nodeName: string
  kind: 'cordon' | 'uncordon' | 'drain' | 'taint-add' | 'taint-remove'
  status: 'confirming' | 'in-progress' | 'complete' | 'cancelled' | 'failed'
  error?: string
  taint?: { key: string; value?: string; effect: string }
  drain?: DrainProgress
  /** The `DrainNode`/`CancelDrainNode` command's request id, stringified (drain only). */
  requestId?: string
}

/**
 * One Helm release uninstall in flight (006-helm-mutate US3, T034/T035), keyed by
 * `namespace/release`. Ephemeral, frontend-held UI state driven by `HelmMutationApplied`/
 * `HelmMutationFailed` (`store/handlers/system.ts`) rather than the generic `CommandFinished`,
 * since those events carry the actual mutate-flow outcome. Success also removes the release from
 * `lists` and closes a matching open Inspector, so a 'complete' entry is normally short-lived.
 */
export interface HelmUninstallState {
  status: 'in-progress' | 'complete' | 'failed'
  error?: string
}

/**
 * One container's on-demand image inspection (010-container-image-inspection), keyed by
 * `${namespace}/${pod}/${container}` (`imageInspectKey`, `store/handlers/imageInspect.ts`).
 * Driven entirely by `ContainerImageInspected`/`ContainerImageInspectFailed` events — there is
 * no local setter; `ImageInspectPanel` only ever reads this and sends `InspectContainerImage`
 * on mount/container-change (never eagerly, FR-003).
 */
/** 021-trivy-vulnerability-pane: one Pod's vulnerability inspection, keyed `${namespace}/${pod}`. */
export interface PodVulnInspectState {
  status: 'idle' | 'loading' | 'ready' | 'error'
  data?: PodVulnerabilityInspection
  error?: VulnInspectError
}

export interface ImageInspectState {
  status: 'loading' | 'ready' | 'error'
  inspection?: ImageInspection
  error?: ImageInspectError
  updatedAt: number
}

export interface ObservabilitySettingsEntry {
  prometheus: PrometheusSettings | null
  grafana: GrafanaSettings | null
}

export interface IntegrationsState {
  prometheusTest?: { success: boolean; message: string }
  grafanaTest?: { success: boolean; message: string }
  /** 039: last `ObservabilitySettings` per context; absent until the backend has answered. */
  settings?: Record<string, ObservabilitySettingsEntry>
  portAvailability: Record<number, boolean>
}

/**
 * 037-node-shell. `session` is the singleton debug-pod session as last reported by
 * `NodeShellStatus` (stale generations are ignored); `access` is the latest permission check;
 * `orphans` the latest leftover-pod sweep; `settings` the per-context defaults.
 */
export interface NodeShellSession {
  sessionId: number
  phase: NodeShellPhase
  node: string
  namespace: string
  pod: string
  detail?: string
  leftover: boolean
}

export interface NodeShellState {
  session?: NodeShellSession
  access?: { namespace: string; checks: NodeShellAccessCheck[] }
  orphans?: OrphanedNodeShellPod[]
  settings?: NodeShellSettings
  settingsContext?: string
}

/**
 * 014-teams-organizations. `clusterSettings`/`clusterSettingsContext`/`clusterSettingsError`
 * reflect the most recent `LoadClusterSettings`/`SaveClusterSettings` round-trip for whichever
 * context requested it — `ClusterSettingsPanel` re-requests on context switch, so stale
 * cross-context reads are never shown. `orgAvailable` is the pending org-level default the
 * operator has not yet accepted/dismissed for `clusterSettingsContext`, if any (FR-007).
 * `rejection` is the last `TeamPublishRejected` reason (suspected secret, conflict, or
 * unreachable store), surfaced by both the settings and profiles panels then cleared locally
 * once shown.
 */
export interface TeamState {
  clusterSettings?: ClusterSettings
  clusterSettingsContext?: string
  clusterSettingsError?: string
  orgAvailable?: OrgClusterSettings
  storeStatus?: TeamStoreStatus
  adopted: AdoptedConnectionProfile[]
  available: ConnectionProfile[]
  rejection?: TeamPublishRejection
}

/**
 * One CRD kind's last-fetched instance list (US3, data-model.md "Custom Resource Instance").
 * `items` are raw JSON strings, one per instance — the same `raw_json`-as-`String` convention
 * every built-in kind's details carry, parsed on demand for display.
 */
export interface CrdInstancesState {
  /** 'unavailable': a fresh `CrdCatalogUpdated` no longer lists this kind (deleted CRD, C1). */
  status: 'ready' | 'unavailable'
  namespace: string | null
  items: readonly string[]
  updatedAt: number
}

export interface CrdState {
  catalog: readonly CrdCatalogEntry[]
  /** True once the first `CrdCatalogUpdated` has arrived (distinguishes "empty" from "loading"). */
  catalogLoaded: boolean
  selected?: { group: string; kind: string; version: string }
  /** Keyed `${group}/${kind}`. */
  instances: Record<string, CrdInstancesState>
}

/** 026-mutation-audit-log-viewer: the local mutation history (newest first) + transient filter. */
export interface AuditState {
  entries: AuditLogEntry[]
  loaded: boolean
  /** True while the Audit Log pane is mounted: completed mutations then trigger a refresh. */
  visible: boolean
  /** Request id of the last `AuditLogLoaded`, so its own `CommandFinished` never re-refreshes. */
  lastLoadedRequest?: number
  filter: AuditLogFilter
}

export interface PrefsState {
  /** True once `UiConfigLoaded` has arrived. */
  loaded: boolean
  expandedGroups: string[]
  /** `auto_start.mcp_stdio`: whether the stdio MCP server (015) is enabled. Off by default. */
  mcpStdioEnabled: boolean
  /** Bumped each time a saved `ui` config is applied, so the slot grid re-seeds its sizes. */
  uiEpoch?: number
}

/** One in-flight or just-finished kind create/delete, keyed by its client `request_id`. */
export interface KindOperation {
  requestId: string
  clusterName: string
  kind: 'create' | 'delete'
  /** Latest stage line from kind's own output. */
  stage?: string
  status: 'in-progress' | 'complete' | 'failed' | 'cancelled'
  error?: string
}

/** 030-kind-cluster-provisioning: state behind the Local Clusters panel. */
export interface KindState {
  /** `undefined` until the first `KindPrerequisitesUpdated`. */
  prerequisites?: KindPrerequisites
  /** `undefined` until the first `KindClustersUpdated`. */
  clusters?: KindClusterSummary[]
  operations: Record<string, KindOperation>
}

/**
 * 015-mcp-agent-exposure: the stdio MCP server's operator-visible status, kept in sync by
 * `McpServerStatusChanged` (`store/handlers/system.ts`) — the reply to `LoadMcpServerStatus`
 * and the broadcast after every `SetMcpServerEnabled`.
 */
export interface McpState {
  /** True once the first `McpServerStatusChanged` has arrived. */
  loaded: boolean
  /** The persisted operator preference (`auto_start.mcp_stdio`). Off by default (FR-002). */
  enabled: boolean
  /** Whether a stdio server task actually exists right now. Never `true` while `enabled` is false. */
  running: boolean
  /** The literal command an external MCP client should launch (FR-007). */
  connectionCommand: string
  /** 015 US4: the "Launch example chat" subprocess (`McpDemoChat*` events). */
  demoChat: McpDemoChatState
}

/**
 * 015-mcp-agent-exposure US4: the locally-launched `claude` CLI session.
 *
 * `'not-detected'` is only ever set by the backend's own `McpDemoChatNotDetected` — the
 * frontend never guesses whether a CLI exists, so the panel can never offer a launch it has
 * been told is impossible, nor claim a detection Fathom did not make (FR-009/FR-011).
 */
export interface McpDemoChatState {
  /** `'idle'`: nothing launched yet (and nothing known to be missing). */
  status: 'idle' | 'running' | 'ended' | 'not-detected'
  /** The backend's monotonic session id; chunks from older sessions are ignored. */
  sessionId?: number
  /** Why the session ended, shown verbatim. */
  reason?: string
  /** The backend's setup text for the `'not-detected'` state. */
  instructions?: string
  /**
   * Decoded output chunks, bounded like `ExecStreamState.chunks` — a short-lived handoff
   * queue `DemoChatTerminal` drains into xterm.js, which owns the rendered scrollback.
   */
  chunks: RingBuffer<string>
  /** Bumped on every appended chunk; the buffer itself is mutated in place. */
  rev: number
}

export interface LayoutState {
  widthClass: WidthClass
  ultraPlus: boolean
  profile: LayoutProfile
  /** Pane shown in each visible slot, index-aligned with `profile.slots`. */
  slotPanes: PaneId[]
  /** Pane shown in each dock tab, index-aligned with `profile.dock.tabs`. */
  dockPanes: PaneId[]
  /**
   * Phone navigation stack; the last entry is on screen. On tablet the root entry picks the
   * active tab (a tool pane, or anything else for the list/inspector split).
   */
  stack: PaneId[]
  /** Navigator drawer (phone/tablet, where nav has no slot). */
  navOpen: boolean
  /** The list pane the operator last worked in; the navigator retargets it. */
  focusedList?: PaneId
}

/** Transient shell overlays (032-ui-shell-a11y); never persisted. */
export interface ShellUi {
  paletteOpen: boolean
  fleetOpen: boolean
  askAiOpen: boolean
  /** Ask AI sheet shows the assistant setup form instead of the conversation. */
  askAiSetup: boolean
  /** Laptop "Tools" overlay sheet: the tool shown in it, or undefined when closed. */
  toolsSheet: ToolKind | undefined
  /** Phone bottom-bar "More" sheet. */
  moreOpen: boolean
  /** Narrow context/namespace sheet. */
  scopeOpen: boolean
}

/** The data part of the store (everything `ingest` may replace). */
export interface StoreData {
  conn: ConnState
  session: SessionState
  /** 036-incluster-daemon: per-context daemon status, timeline, topology, metrics. */
  daemon: DaemonSlice
  lists: Record<ListKey, ListState>
  details: Record<DetailKey, DetailState>
  /** Log and exec streams by `StreamId`. */
  streams: Record<number, LogStreamState | ExecStreamState>
  /** Port-forward tunnels by tunnel id. */
  tunnels: Record<string, TunnelState>
  /** 009: decorative Service-origin tags for forwards, by tunnel id. */
  portForwardOrigins: Record<string, PortForwardOrigin>
  /** 009: Service-launched starts awaiting a matching `PortForwardsUpdated` row. */
  pendingPortForwardOrigins: PendingPortForwardOrigins
  metrics: MetricsState
  flux: { overview?: FluxOverview }
  knative: { overview?: KnativeOverview }
  /** 019: `undefined` until the first `ClusterVersionUpdated` arrives. */
  clusterVersion: ClusterVersionSummary | undefined
  gatekeeper: { overview?: GatekeeperOverview }
  velero: { overview?: VeleroOverview }
  certManager: { overview?: CertManagerOverview; chain?: CertificateChain }
  /** 021-trivy-vulnerability-pane: cluster-wide Trivy Operator summary. */
  trivy: { overview?: TrivyOverview }
  hubble: { summary?: HubbleTrafficSummary }
  keda: { overview?: KedaOverview; selected?: KedaSelection }
  crds: CrdState
  integrations: IntegrationsState
  /** Node actions (US2), by node name. */
  nodeActions: Record<string, NodeAction>
  /**
   * Container image inspections (010-container-image-inspection), by
   * `${namespace}/${pod}/${container}`. See `ImageInspectState`.
   */
  imageInspections: Record<string, ImageInspectState>
  /** Pod vulnerability inspections by `${namespace}/${pod}` (021-trivy-vulnerability-pane). */
  podVulnInspections: Record<string, PodVulnInspectState>
  /**
   * 013-agent-sdk-ops: per-object assistant conversations, keyed by
   * `${kind}/${namespace}/${name}` (matches the backend's `conversation_key`). Driven entirely
   * by `AgentConversationUpdated` (`store/handlers/agent.ts`) — its payload has no top-level
   * `namespace`/`name`, so it is special-cased in `ingest.ts` like `ContainerImageInspected`.
   */
  agentConversations: Record<string, AgentConversation>
  /**
   * 013-agent-sdk-ops (US2/P1): the operator's global assistant auth mode, plus a key the
   * operator explicitly revealed via `AgentRevealKey`. Global (not keyed), same as
   * `flux`/`hubble`/`helmMutation` — there is exactly one auth mode for the whole app, not one
   * per object. `mode` is `undefined` until the first `AgentGetAuthMode`/`AgentSetAuthMode`
   * reply arrives (`store/handlers/system.ts`'s `AgentAuthModeUpdated` case).
   * `revealedKey` is set only by an explicit `AgentKeyRevealed` reply and is never populated by
   * anything else — `AuthSetup.tsx` never dispatches `AgentRevealKey` on mount/mode-select, only
   * on an explicit operator click (FR-004, Principle VI).
   */
  agentAuth: { mode?: AgentAuthMode; revealedKey?: string }
  /**
   * 038-sdk-login-ollama: provider login/detection state, sourced only from backend
   * `Ai*` events. `loaded` flips on the first `AiProviderStatus`. Never holds a secret.
   */
  ai: AiSlice
  /**
   * 011-rbac-oidc-administration: three read-only RBAC payloads with no `name` field for the
   * generic `details` keying (`readKeyOf` assumes one) — same reasoning as `imageInspections`.
   * Global (not keyed): only one self-access check, namespace-access lookup, or kubeadm
   * detection is ever in flight/shown at a time.
   */
  /** 024-pod-exec-terminal: latest `PodExecAccessChecked` per `execAccessKey`. */
  execAccess: Record<string, ExecAccessCheckState>
  rbacAccess: {
    /** Latest `RbacSelfAccessUpdated` reply (US2/US3 create/edit/delete gating, FR-007). */
    selfAccess?: RbacAccessResult[]
    /** 018: `ResourceMutationAccessChecked` replies keyed by `resourceAccessKey`. */
    resourceAccess?: Record<string, RbacAccessResult>
    /** Latest `NamespaceAccessResolved` reply (US4). */
    namespaceAccess?: NamespaceAccessResolved
    /** Latest `KubeadmDetectionUpdated` reply (US5), never an error. */
    kubeadmDetection?: KubeadmDetectionResult
  }
  /**
   * 006-helm-mutate: the in-flight/last rollback-upgrade-uninstall preview→apply flow. Global
   * (not keyed), same as `flux`/`hubble` — only one Helm mutate flow can be open at a time
   * (research.md R2/R9). `system.ts` advances it as `HelmMutationPreviewReady/Applied/Failed`
   * events arrive. Used by `helm.tsx`'s per-row rollback action.
   */
  helmMutation: HelmMutationSession
  /**
   * 014-teams-organizations: local §10 Cluster Settings (+ org-sync affordances) and shared,
   * non-secret connection profiles. Global (not per-context) for the profile/store-status
   * fields — mirrors `flux`/`hubble` — but `clusterSettings`/`clusterSettingsContext` track
   * which context the last `LoadClusterSettings`/`ClusterSettingsLoaded` round-trip was for,
   * since §10 settings are themselves per-context (`store/handlers/system.ts`'s
   * `ClusterSettingsLoaded`/`ClusterSettingsError`/`TeamStoreStatusUpdated`/
   * `ConnectionProfilesUpdated`/`OrgClusterSettingsAvailable`/`TeamPublishRejected` cases).
   */
  team: TeamState
  /** 033 US1/US2: OIDC issuers, signed-in identities and sign-in progress. */
  identity: IdentityState
  /** 033 US3: IdP directory snapshots, admin-key presence and advisory team links. */
  teams: TeamsState
  /** 033 US4: role-assignment preview, orphaned subjects, IdP directory cache. */
  roles: RolesState
  /** 037-node-shell: debug-pod session, access check, orphan sweep and settings. */
  nodeShell: NodeShellState
  /** Helm release uninstalls (006 US3), by `namespace/release`. */
  helmUninstalls: Record<string, HelmUninstallState>
  /**
   * Helm upgrade preview + apply state (006-helm-mutate), keyed by `${namespace}/${release}` —
   * one session per release, mirroring `nodeActions`'s per-node shape. Used by
   * `HelmUpgradePanel.tsx`'s Inspector "Upgrade" tab.
   */
  helmMutations: Record<string, HelmMutationSession>
  layout: LayoutState
  shell: ShellUi
  panes: Record<PaneId, PaneState>
  /**
   * In-progress YAML edits (US1, data-model.md "Resource Edit Session"), keyed like `details`.
   * Ephemeral: only entries for objects an Inspector's YAML tab is (or was) open on.
   */
  editSessions: Record<DetailKey, ResourceEditSession>
  /** Multi-select for bulk actions, by pane. */
  selection: Record<PaneId, string[]>
  /** Actions awaiting a result, by request id. */
  pending: Record<number, PendingAction>
  /** The action dialog on screen, if any. */
  actionPrompt?: ActionPrompt
  ui: UiConfig
  prefs: PrefsState
  /** 031-hosted-server-mode: shell mode, signed-in identity, limits and terminal states. */
  hosted: HostedState
  audit: AuditState
  /** 015-mcp-agent-exposure: stdio MCP server status, shown in Cluster Settings. */
  mcp: McpState
  /** 030-kind-cluster-provisioning: local kind cluster readiness, inventory, and operations. */
  kind: KindState
  /**
   * 008-pod-lifecycle-diagnostics: a "view logs for this container" click from the Lifecycle
   * section's Init Containers list, so the Inspector can switch to the Logs tab and
   * `PodLogsTab` can pre-select the container. Cleared once `PodLogsTab` consumes it.
   */
  podLogsIntent: { namespace: string; name: string; container: string } | null
}

export interface StoreState extends StoreData {
  /** Fire-and-forget command sink; `useBackend` installs the transport-backed one. */
  send: (cmd: BackendCmd) => void
  /**
   * Send a command and resolve to its `RequestId` (the key of its `CommandFinished`). Rejects
   * if the backend is gone. `useBackend` installs the transport-backed one.
   */
  request: (cmd: BackendCmd) => Promise<number>

  setWidthClass: (cls: WidthClass, ultraPlus?: boolean) => void
  /** Persist a resized/rearranged profile for the current class. */
  setProfile: (profile: LayoutProfile) => void
  setTheme: (patch: Partial<ThemePrefs>) => void
  /** Set (or clear with `undefined`) the density override; persisted via `SaveUiConfig`. */
  setDensityOverride: (density: Density | undefined) => void
  /** 026: refresh the audit log, and track whether its pane is visible. */
  refreshAuditLog: () => void
  setAuditVisible: (visible: boolean) => void
  setAuditFilter: (patch: Partial<AuditLogFilter>) => void
  updatePane: (id: PaneId, patch: Partial<Omit<PaneState, 'id' | 'kind'>>) => void
  pushPane: (id: PaneId) => void
  /** Phone: make the pane for `kind`/`tool` the root of the stack, creating it if needed. */
  openRoot: (kind: PaneKind, tool?: ToolKind) => void
  /** Show `section` in the first visible list pane (the phone's list pane when narrow). */
  navigate: (section: SectionKey) => void
  popPane: () => void
  setNavOpen: (open: boolean) => void
  setShell: (patch: Partial<ShellUi>) => void
  /** Find or create the (off-layout) pane hosting `tool`, for overlay sheets. */
  ensureToolPane: (tool: ToolKind) => PaneId
  setConnection: (patch: Partial<ConnState>) => void
  notify: (level: Notice['level'], message: string) => void
  /** Switch kubeconfig context: sends `SetContext`; results for other contexts are then dropped. */
  selectContext: (name: string) => void
  /** 027: start/stop the backend's view-scoped fleet pollers (Fleet Dashboard mount/unmount). */
  startFleetPolling: () => void
  stopFleetPolling: () => void
  /** 034: add/remove a context to the live federated set; re-request state on mount. */
  federateContext: (context: string) => void
  unfederateContext: (context: string) => void
  requestFederationSnapshot: () => void
  /** 036: opt a context in/out of the in-cluster daemon; manual re-probe; metrics on demand. */
  setDaemonPreference: (context: string, mode: 'off' | 'auto') => void
  refreshDaemon: (context: string) => void
  requestDaemonTopology: (context: string) => void
  requestDaemonMetrics: (context: string, uid: string, metric: 'cpu' | 'memory') => void
  /** Ask the backend to re-emit its buffered timeline (newest `limit` entries). */
  requestDaemonTimeline: (context: string, limit: number) => void
  /** Sends `SetNamespaceFilter` (`null` = all namespaces). */
  selectNamespace: (ns: string | null) => void
  /** Sends the section's `Refresh*` command and marks its list loading. */
  refreshSection: (section: SectionKey) => void
  /** Sends `Select<Kind>` to read one object's details (and its `raw_json`/YAML). */
  inspect: (kind: string, namespace: string, name: string) => void
  /** Sends `RefreshEvents` (cluster events; the inspector filters by involved object). */
  refreshEvents: () => void
  /** Sends `RefreshFlux` (rescans GitOps Toolkit resources; also polled every 20s). */
  refreshFlux: () => void
  /** Sends `RefreshKnative` (rescans Knative Serving resources; also polled every 20s). */
  refreshKnative: () => void
  /** Sends `RefreshGatekeeper` (rescans OPA Gatekeeper constraints; also polled every 20s). */
  refreshGatekeeper: () => void
  /** Sends `RefreshVelero` (rescans Velero backup resources; also polled every 20s). */
  refreshVelero: () => void
  /** Sends `RefreshCertManager` (rescans Certificates; also polled every 20s). */
  refreshCertManager: () => void
  /** Sends `SelectCertificateChain` (resolves one Certificate's lifecycle chain). */
  selectCertificateChain: (namespace: string, name: string) => void
  /** Sends `RefreshTrivyOverview` (rescans Trivy reports; also polled every 20s). */
  refreshTrivyOverview: () => void
  /** Sends `InspectPodVulnerabilities` and marks the Pod's slice loading. */
  inspectPodVulnerabilities: (namespace: string, pod: string) => void
  /** Sends `RefreshKeda` (rescans KEDA ScaledObjects/ScaledJobs; also polled every 20s). */
  refreshKeda: () => void
  /** Sends `SelectScaledObject`/`SelectScaledJob` for the Autoscalers Inspector. */
  selectKedaResource: (kind: string, namespace: string, name: string) => void
  /** Sends `RefreshHubble` (rescans Hubble traffic; also polled every 20s). */
  refreshHubble: () => void
  /**
   * US3: select one CRD kind (`SelectCrdKind`) and immediately fetch its instances
   * (`RefreshCrdInstances`); records the selection so a later `CrdCatalogUpdated` that drops
   * this kind can mark it unavailable (C1).
   */
  selectCrdKind: (entry: CrdCatalogEntry) => void
  /** Remember which list pane the navigator should retarget. */
  focusList: (id: PaneId) => void
  /**
   * 008-pod-lifecycle-diagnostics: request that the Inspector switch to the Logs tab and
   * pre-select `container` (used by the Lifecycle section's Init Containers logs affordance).
   */
  requestPodLogs: (namespace: string, name: string, container: string) => void
  /** Clears `podLogsIntent` once `PodLogsTab` has consumed it. */
  clearPodLogsIntent: () => void
  /**
   * 009: record that a `StartPortForward` just dispatched from a Service inspector targeted
   * `namespace/pod/localPort/remotePort`, so the next matching `PortForwardsUpdated` row gets
   * tagged with `service` in `portForwardOrigins` (`store/handlers/list.ts`).
   */
  recordPortForwardOrigin: (target: {
    namespace: string
    pod: string
    localPort: number
    remotePort: number
    service: string
  }) => void
  /**
   * Show `ref` in the nearest inspector (pushing one on phone), mark it selected in the `from`
   * list pane, and send the descriptor's read commands. `push` (US3, research.md R6): true when
   * following a related-resource link from within an inspector (the current `target` is pushed
   * onto `history`); false/omitted when selecting fresh from a list row (`history` is cleared).
   */
  openInspector: (ref: ObjectRef, from?: PaneId, push?: boolean) => void
  /**
   * US3: pop the last entry off `paneId`'s `history` and restore it as `target`, without
   * re-issuing a fetch (the detail cache already holds it). No-op when `history` is empty.
   */
  goBack: (paneId: PaneId) => void
  /** Esc: pop the inspector on phone, clear it elsewhere. */
  closeInspector: () => void
  /**
   * US1 (YAML edit + apply): open (or reset) the edit session for `ref`, seeded from `raw`
   * (the same `raw_json` the read-only YAML tab renders).
   */
  openEditSession: (ref: ObjectRef, raw: string) => void
  /**
   * 011-rbac-oidc-administration US2: open a create-mode session for a not-yet-existing object
   * (e.g. a new RoleBinding built by `RoleBindingForm`), keyed the same as an edit session so
   * the rest of the diff/apply machinery (`updateEditedYaml`/`reviewEditDiff`/`applyEditSession`)
   * needs no changes. Returns the session's `DetailKey` so the caller can look it up.
   */
  openCreateEditSession: (target: ResourceEditTarget, initialYaml: string) => DetailKey
  /** Update the operator's edit buffer for the session at `key` (`detailKey(...)`). */
  updateEditedYaml: (key: DetailKey, yaml: string) => void
  /** Open the diff view and mark the current `liveYaml` as reviewed. */
  reviewEditDiff: (key: DetailKey) => void
  /** Discard the edit buffer back to the current `liveYaml` (T026: navigate-away, or Discard). */
  discardEditSession: (key: DetailKey) => void
  /** Drop the session entirely (the Inspector's YAML tab is no longer showing this object). */
  closeEditSession: (key: DetailKey) => void
  /**
   * Send `ApplyResourceYaml` for the session at `key` and await its `CommandFinished`. Success
   * is reflected via the `ResourceApplied` stream event (handlers/stream.ts); this only needs
   * to classify a failure into `'conflict'` | `'deleted'` | `'error'` (data-model.md).
   */
  applyEditSession: (key: DetailKey) => Promise<void>
  /** Toggle one row in a pane's multi-select (bulk actions). */
  toggleRowSelected: (paneId: PaneId, rowKey: string) => void
  /** Replace a pane's multi-select (`[]` clears it). */
  setSelection: (paneId: PaneId, rowKeys: string[]) => void
  /** Show the confirm/scale dialog for an action (US3). Nothing is sent until it is confirmed. */
  openAction: (prompt: ActionPrompt) => void
  /** Dismiss the action dialog (Esc, Cancel, or after confirming). */
  closeAction: () => void
  /** Expand/collapse navigator groups; persisted with a debounced `SaveUiConfig`. */
  setExpandedGroups: (groups: string[]) => void
  /** Set (or clear, with `undefined`) the in-flight/last node action for `nodeName` (US2). */
  setNodeAction: (nodeName: string, action: NodeAction | undefined) => void
  /**
   * Sends `PreviewHelmRollback` and marks the flow `previewing`; the reply arrives as
   * `HelmMutationPreviewReady`/`HelmMutationFailed` (`system.ts`), not through this call.
   */
  previewHelmRollback: (namespace: string, release: string, targetRevision: number) => void
  /**
   * Sends `ApplyHelmRollback` and marks the flow `applying`; the reply arrives as
   * `HelmMutationApplied`/`HelmMutationFailed` (`system.ts`), not through this call.
   */
  applyHelmRollback: (
    namespace: string,
    release: string,
    targetRevision: number,
    baseRevision: number,
  ) => void
  /** Back to `idle` (Cancel, or navigating away from the release). */
  resetHelmMutation: () => void
  /** Set (or clear, with `undefined`) the in-flight/last uninstall for `namespace/release` (006 US3). */
  setHelmUninstall: (key: string, state: HelmUninstallState | undefined) => void
  /** Set (or clear, with `undefined`) a release's Helm mutation session (006-helm-mutate). */
  setHelmMutation: (key: string, session: HelmMutationSession | undefined) => void
}

/** Debounce for persisting navigator state (typing-speed toggles coalesce into one write). */
export const SAVE_DEBOUNCE_MS = 500
let saveTimer: ReturnType<typeof setTimeout> | undefined

/** Debounced `SaveUiConfig` of the current `ui` prefs + expanded groups. */
function scheduleUiSave(get: () => StoreState): void {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    const s = get()
    s.send({ type: 'SaveUiConfig', data: { ui: s.ui, expanded_groups: s.prefs.expandedGroups } })
  }, SAVE_DEBOUNCE_MS)
}

/** Lists the backend refreshes after a context or namespace change (src/backend.rs). */
export const CONTEXT_LISTS: readonly SectionKey[] = [SECTION.pods, SECTION.deployments]

export function markLoading(
  lists: Record<ListKey, ListState>,
  context: string | undefined,
  sections: readonly SectionKey[],
): Record<ListKey, ListState> {
  if (context === undefined) return lists
  const next = { ...lists }
  for (const section of sections) {
    // The backend reports namespaced lists with namespace `null` and filters server-side;
    // cluster-scoped kinds (e.g. nodes) key under `Cluster` instead.
    const scope = descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced'
    const key = listKey(context, section, scope, null)
    const prev = next[key]
    next[key] = prev
      ? { ...prev, status: 'loading', error: undefined }
      : { rows: [], status: 'loading', updatedAt: 0 }
  }
  return next
}

// ------------------------------------------------------------------ pane assignment

/** What a fresh list pane shows: pods in the first, deployments in the second (MVP kinds). */
const DEFAULT_LIST_SECTIONS: readonly SectionKey[] = [SECTION.pods, SECTION.deployments]

function defaultRoute(id: PaneId): Route | undefined {
  const n = Number(/^list-(\d+)$/.exec(id)?.[1] ?? NaN)
  const section = DEFAULT_LIST_SECTIONS[n - 1]
  return section ? { section, scope: 'Namespaced', context: null, namespace: null } : undefined
}

function newPane(id: PaneId, kind: PaneKind, tool?: ToolKind): PaneState {
  const route = kind === 'list' ? defaultRoute(id) : undefined
  return {
    id,
    kind,
    ...(tool ? { tool } : {}),
    ...(route ? { route } : {}),
    search: '',
    scrollTop: 0,
  }
}

function nextId(panes: Record<PaneId, PaneState>, prefix: string): PaneId {
  let n = 1
  while (`${prefix}-${n}` in panes) n++
  return `${prefix}-${n}`
}

/**
 * Map each slot to a pane: reuse the first unused pane of the same kind (and tool), else create
 * one. Panes that no longer have a slot are kept, so widening again restores their state.
 */
export function assignPanes(
  profile: LayoutProfile,
  existing: Record<PaneId, PaneState>,
): { slotPanes: PaneId[]; dockPanes: PaneId[]; panes: Record<PaneId, PaneState> } {
  const panes = { ...existing }
  const used = new Set<PaneId>()
  const claim = (kind: PaneKind, tool?: ToolKind, prefix: string = tool ?? kind): PaneId => {
    const found = Object.values(panes).find(
      (p) => !used.has(p.id) && p.kind === kind && p.tool === tool && p.id.startsWith(prefix),
    )
    const id = found?.id ?? nextId(panes, prefix)
    if (!found) panes[id] = newPane(id, kind, tool)
    used.add(id)
    return id
  }
  const slotPanes = profile.slots.map((s) =>
    claim(s.kind, s.kind === 'tool' ? (s.tool ?? undefined) : undefined),
  )
  const dockPanes = (profile.dock?.tabs ?? []).map((tool) => claim('tool', tool, `dock-${tool}`))
  return { slotPanes, dockPanes, panes }
}

// ------------------------------------------------------------------ defaults

export const DEFAULT_THEME: ThemePrefs = {
  mode: 'dark',
  accent: 'cyan',
  effects: 'on',
  glitch: false,
}

export const DEFAULT_UI: UiConfig = { schema: 1, theme: DEFAULT_THEME, density: {}, layouts: {} }

/** A fresh copy of the store's data (also used by tests to reset between cases). */
export function initialData(): StoreData {
  const profile = resolveProfile('phone')
  const { slotPanes, dockPanes, panes } = assignPanes(profile, {})
  return {
    conn: { status: 'starting', dropped: 0 },
    daemon: initialDaemon(),
    session: {
      contexts: [],
      contextsLoaded: false,
      namespaces: [],
      activeNamespace: null,
      namespaceFilter: [],
      contextStatuses: {},
      cloudAuth: {},
      fleetSummaries: {},
      federation: initialFederation(),
    },
    lists: {},
    details: {},
    streams: {},
    tunnels: {},
    portForwardOrigins: {},
    pendingPortForwardOrigins: {},
    metrics: { diskUsage: [] },
    flux: {},
    knative: {},
    clusterVersion: undefined,
    gatekeeper: {},
    velero: {},
    certManager: {},
    trivy: {},
    hubble: {},
    keda: {},
    crds: { catalog: [], catalogLoaded: false, instances: {} },
    integrations: { portAvailability: {} },
    nodeActions: {},
    imageInspections: {},
    podVulnInspections: {},
    agentConversations: {},
    agentAuth: {},
    ai: { loaded: false, statuses: [], active: null, offline: false, ollamaModel: null },
    team: { adopted: [], available: [] },
    identity: initialIdentityState(),
    teams: initialTeamsState(),
    roles: initialRolesState(),
    nodeShell: {},
    execAccess: {},
    rbacAccess: {},
    helmUninstalls: {},
    helmMutations: {},
    layout: {
      widthClass: 'phone',
      ultraPlus: false,
      profile,
      slotPanes,
      dockPanes,
      stack: slotPanes.slice(0, 1),
      navOpen: false,
    },
    shell: {
      paletteOpen: false,
      fleetOpen: false,
      askAiOpen: false,
      askAiSetup: false,
      toolsSheet: undefined,
      moreOpen: false,
      scopeOpen: false,
    },
    panes,
    editSessions: {},
    selection: {},
    pending: {},
    actionPrompt: undefined,
    helmMutation: initialHelmMutationSession,
    ui: DEFAULT_UI,
    prefs: { loaded: false, expandedGroups: [], mcpStdioEnabled: false },
    hosted: { mode: 'desktop' },
    audit: { entries: [], loaded: false, visible: false, filter: DEFAULT_AUDIT_FILTER },
    mcp: {
      loaded: false,
      enabled: false,
      running: false,
      connectionCommand: '',
      demoChat: { status: 'idle', chunks: new RingBuffer<string>(), rev: 0 },
    },
    kind: { prerequisites: undefined, clusters: undefined, operations: {} },
    podLogsIntent: null,
  }
}

export const useStore = create<StoreState>()((set, get) => ({
  ...initialData(),

  send: () => undefined,
  request: () => Promise.reject(new Error('Not connected to the Fathom backend yet.')),

  setWidthClass: (cls, ultraPlus = false) =>
    set((s) => {
      if (s.layout.widthClass === cls && s.layout.ultraPlus === ultraPlus) return s
      const profile = resolveProfile(cls, s.ui.layouts[cls], ultraPlus)
      const { slotPanes, dockPanes, panes } = assignPanes(profile, s.panes)
      const stack = s.layout.stack.filter((id) => id in panes)
      return {
        panes,
        layout: {
          widthClass: cls,
          ultraPlus,
          profile,
          slotPanes,
          dockPanes,
          stack: stack.length > 0 ? stack : slotPanes.slice(0, 1),
          navOpen: false,
        },
      }
    }),

  setProfile: (saved) => {
    set((s) => {
      const cls = s.layout.widthClass
      const profile = resolveProfile(cls, saved, s.layout.ultraPlus)
      const { slotPanes, dockPanes, panes } = assignPanes(profile, s.panes)
      return {
        panes,
        layout: { ...s.layout, profile, slotPanes, dockPanes },
        ui: { ...s.ui, layouts: { ...s.ui.layouts, [cls]: profile } },
      }
    })
    // Only user-initiated changes reach here (drag end, pin/swap), so it is safe to persist.
    scheduleUiSave(get)
  },

  refreshAuditLog: () => {
    get().send({ type: 'RefreshAuditLog' })
  },
  setAuditVisible: (visible) => set((s) => ({ audit: { ...s.audit, visible } })),
  setAuditFilter: (patch) =>
    set((s) => ({ audit: { ...s.audit, filter: { ...s.audit.filter, ...patch } } })),
  setTheme: (patch) => {
    set((s) => ({ ui: { ...s.ui, theme: { ...s.ui.theme, ...patch } } }))
    scheduleUiSave(get)
  },
  setDensityOverride: (density) => {
    set((s) => ({
      ui: {
        ...s.ui,
        density: density === undefined ? {} : { ...s.ui.density, override: density },
      },
    }))
    scheduleUiSave(get)
  },

  updatePane: (id, patch) =>
    set((s) => {
      const pane = s.panes[id]
      return pane ? { panes: { ...s.panes, [id]: { ...pane, ...patch } } } : s
    }),

  pushPane: (id) =>
    set((s) =>
      s.layout.stack.at(-1) === id
        ? s
        : { layout: { ...s.layout, stack: [...s.layout.stack, id] } },
    ),

  openRoot: (kind, tool) =>
    set((s) => {
      const prefix = tool ?? kind
      const found = Object.values(s.panes).find(
        (p) => p.kind === kind && p.tool === tool && p.id.startsWith(`${prefix}-`),
      )
      const id = found?.id ?? nextId(s.panes, prefix)
      return {
        panes: found ? s.panes : { ...s.panes, [id]: newPane(id, kind, tool) },
        layout: { ...s.layout, stack: [id] },
      }
    }),

  navigate: (section) => {
    set((s) => {
      s = { ...s, conn: withoutNotice(s.conn) }
      const focused = s.layout.focusedList
      const listId =
        (focused !== undefined &&
        s.panes[focused]?.kind === 'list' &&
        (s.layout.slotPanes.includes(focused) || s.layout.stack.includes(focused))
          ? focused
          : undefined) ??
        s.layout.slotPanes.find((id) => s.panes[id]?.kind === 'list') ??
        Object.values(s.panes).find((p) => p.kind === 'list')?.id
      const pane = listId ? s.panes[listId] : undefined
      if (!pane) return s
      const route: Route = {
        context: s.session.activeContext ?? null,
        section,
        scope: 'Namespaced',
        namespace: s.session.activeNamespace,
      }
      return {
        conn: s.conn,
        panes: {
          ...s.panes,
          [pane.id]: {
            ...pane,
            route,
            selection: undefined,
            focusedRow: undefined,
            sort: undefined,
            scrollTop: 0,
          },
        },
        layout: {
          ...s.layout,
          navOpen: false,
          focusedList: pane.id,
          stack:
            s.layout.widthClass === 'phone' || s.layout.widthClass === 'tablet'
              ? [pane.id]
              : s.layout.stack,
        },
      }
    })
    get().refreshSection(section)
  },

  popPane: () =>
    set((s) =>
      s.layout.stack.length <= 1
        ? s
        : { layout: { ...s.layout, stack: s.layout.stack.slice(0, -1) } },
    ),

  setNavOpen: (open) => set((s) => ({ layout: { ...s.layout, navOpen: open } })),

  setShell: (patch) => set((s) => ({ shell: { ...s.shell, ...patch } })),

  ensureToolPane: (tool) => {
    const existing = Object.values(get().panes).find(
      (p) => p.kind === 'tool' && p.tool === tool && p.id.startsWith(`sheet-${tool}`),
    )
    if (existing) return existing.id
    const id = nextId(get().panes, `sheet-${tool}`)
    set((s) => ({ panes: { ...s.panes, [id]: newPane(id, 'tool', tool) } }))
    return id
  },

  setConnection: (patch) => set((s) => ({ conn: { ...s.conn, ...patch } })),

  notify: (level, message) =>
    set((s) => ({ conn: { ...s.conn, lastNotice: { level, message, at: Date.now() } } })),

  startFleetPolling: () => get().send({ type: 'StartFleetPolling' }),

  stopFleetPolling: () => {
    const s = get()
    s.send({ type: 'StopFleetPolling' })
    set({ session: { ...s.session, fleetSummaries: {} } })
  },

  federateContext: (context) => get().send({ type: 'FederateContext', data: { context } }),
  unfederateContext: (context) => get().send({ type: 'UnfederateContext', data: { context } }),
  requestFederationSnapshot: () => get().send({ type: 'RequestFederationSnapshot' }),

  setDaemonPreference: (context, mode) => {
    const s = get()
    // Optimistic: the toggle reflects the click at once; the backend's status replaces it.
    const prev = s.daemon[context]
    const status: DaemonStatus = {
      context,
      state: mode === 'off' ? 'off' : 'probing',
      detail: null,
      capabilities: null,
      lastEventAt: null,
      nextRetryAt: null,
    }
    set({
      daemon: {
        ...s.daemon,
        [context]: {
          session: 0,
          timeline: [],
          truncated: null,
          topology: { seq: 0, nodes: {}, edges: [] },
          metrics: {},
          ...prev,
          status,
        },
      },
    })
    s.send({ type: 'SetDaemonPreference', data: { context, mode } })
  },
  refreshDaemon: (context) => get().send({ type: 'RefreshDaemon', data: { context } }),
  requestDaemonTopology: (context) =>
    get().send({ type: 'RequestDaemonTopology', data: { context } }),
  requestDaemonTimeline: (context, limit) =>
    get().send({ type: 'RequestDaemonTimeline', data: { context, limit } }),
  requestDaemonMetrics: (context, uid, metric) =>
    get().send({
      type: 'RequestDaemonMetrics',
      data: { context, uid, metric, range_secs: 3600 },
    }),

  selectContext: (name) => {
    const s = get()
    if (s.session.activeContext === name) return
    set({
      conn: withoutNotice(s.conn),
      session: {
        ...s.session,
        activeContext: name,
        activeNamespace: null,
        namespaceFilter: [],
        namespaces: [],
      },
      lists: markLoading(s.lists, name, CONTEXT_LISTS),
      // 018: cached permission answers belong to the previous context's identity.
      rbacAccess: {},
    })
    s.send({ type: 'SetContext', data: { context: name } })
  },

  selectNamespace: (ns) => {
    const s = get()
    set({
      conn: withoutNotice(s.conn),
      session: { ...s.session, activeNamespace: ns },
      lists: markLoading(s.lists, s.session.activeContext, CONTEXT_LISTS),
    })
    s.send(namespaceFilterCmd(ns))
  },

  refreshSection: (section) => {
    const s = get()
    const d = descriptorFor(section)
    const route: Route = {
      section,
      scope: d?.namespaced === false ? 'Cluster' : 'Namespaced',
      context: s.session.activeContext ?? null,
      namespace: s.session.activeNamespace,
    }
    const cmd = d ? d.refresh(route) : refreshCmd(section)
    if (!cmd) return
    set({ lists: markLoading(s.lists, s.session.activeContext, [section]) })
    s.send(cmd)
  },

  inspect: (kind, namespace, name) => {
    const cmd = selectCmd(kind, namespace, name)
    if (cmd) get().send(cmd)
  },

  refreshEvents: () => get().send({ type: 'RefreshEvents' }),

  refreshFlux: () => get().send({ type: 'RefreshFlux' }),

  refreshKnative: () => get().send({ type: 'RefreshKnative' }),
  refreshGatekeeper: () => get().send({ type: 'RefreshGatekeeper' }),
  refreshVelero: () => get().send({ type: 'RefreshVelero' }),
  refreshCertManager: () => get().send({ type: 'RefreshCertManager' }),
  selectCertificateChain: (namespace, name) =>
    get().send({ type: 'SelectCertificateChain', data: { namespace, name } }),

  refreshTrivyOverview: () => get().send({ type: 'RefreshTrivyOverview' }),

  inspectPodVulnerabilities: (namespace, pod) => {
    set((s) => ({
      podVulnInspections: {
        ...s.podVulnInspections,
        [`${namespace}/${pod}`]: { status: 'loading' },
      },
    }))
    get().send({ type: 'InspectPodVulnerabilities', data: { namespace, pod } })
  },
  refreshKeda: () => get().send({ type: 'RefreshKeda' }),

  selectKedaResource: (kind, namespace, name) =>
    get().send({
      type: kind === 'ScaledJob' ? 'SelectScaledJob' : 'SelectScaledObject',
      data: { namespace, name },
    }),

  refreshHubble: () => get().send({ type: 'RefreshHubble' }),

  previewHelmRollback: (namespace, release, targetRevision) => {
    set({ helmMutation: startHelmPreview() })
    get().send({
      type: 'PreviewHelmRollback',
      data: { namespace, release, target_revision: targetRevision },
    })
  },

  applyHelmRollback: (namespace, release, targetRevision, baseRevision) => {
    set((s) => ({ helmMutation: startHelmApply(s.helmMutation) }))
    get().send({
      type: 'ApplyHelmRollback',
      data: {
        namespace,
        release,
        target_revision: targetRevision,
        base_revision: baseRevision,
      },
    })
  },

  resetHelmMutation: () => set({ helmMutation: resetHelmMutationSession() }),

  selectCrdKind: (entry) => {
    const version = entry.versions[0] ?? ''
    set((s) => ({
      crds: { ...s.crds, selected: { group: entry.group, kind: entry.kind, version } },
    }))
    const s = get()
    s.send({
      type: 'SelectCrdKind',
      data: { group: entry.group, kind: entry.kind, version },
    })
    s.send({
      type: 'RefreshCrdInstances',
      data: { group: entry.group, kind: entry.kind, version, namespace: null },
    })
  },

  focusList: (id) =>
    set((s) => (s.layout.focusedList === id ? s : { layout: { ...s.layout, focusedList: id } })),

  requestPodLogs: (namespace, name, container) =>
    set({ podLogsIntent: { namespace, name, container } }),
  clearPodLogsIntent: () => set({ podLogsIntent: null }),

  recordPortForwardOrigin: ({ namespace, pod, localPort, remotePort, service }) =>
    set((s) => ({
      pendingPortForwardOrigins: {
        ...s.pendingPortForwardOrigins,
        [pendingPortForwardOriginKey(namespace, pod, localPort, remotePort)]: service,
      },
    })),

  openInspector: (ref, from, push) => {
    const nextHistory = (pane: PaneState | undefined): ObjectRef[] =>
      push && pane?.target ? [...(pane.history ?? []), pane.target] : []
    set((s) => {
      const panes = { ...s.panes }
      const rowKey = ref.namespace ? `${ref.namespace}/${ref.name}` : ref.name
      const source = from ? panes[from] : undefined
      if (source) panes[source.id] = { ...source, selection: rowKey, focusedRow: rowKey }
      const layout = { ...s.layout, ...(from ? { focusedList: from } : {}) }

      const slotted = s.layout.slotPanes.find((id) => panes[id]?.kind === 'inspector')
      const slottedPane = slotted ? panes[slotted] : undefined
      // The fixed-slot inspector is only a valid target when it's actually on screen at the same
      // time as whatever triggered this: laptop+ always shows the row alongside the dock, but
      // tablet swaps the whole content area between the row and a tool tab (007-topology-graph-
      // view: a topology node click while the Topology tab is active must NOT silently retarget
      // the off-screen Resources-tab inspector slot) — so on tablet the fast path only applies
      // when the click genuinely came from a pane that's part of that visible row.
      const rowAlwaysVisible = s.layout.profile.dock !== undefined
      const fromVisibleSlot = from !== undefined && s.layout.slotPanes.includes(from)
      // A drill-down link (push) is clicked inside an inspector; when that inspector is the
      // slotted one (no inspector on the stack), retargeting it is what the user sees.
      const drillFromSlotted =
        push === true &&
        slottedPane?.target !== undefined &&
        !s.layout.stack.some((id) => panes[id]?.kind === 'inspector')
      if (
        slottedPane &&
        s.layout.widthClass !== 'phone' &&
        (rowAlwaysVisible || fromVisibleSlot || drillFromSlotted)
      ) {
        panes[slottedPane.id] = {
          ...slottedPane,
          target: ref,
          source: from,
          scrollTop: 0,
          history: nextHistory(slottedPane),
        }
        return { panes, layout }
      }
      // Phone: drill in. Reuse the inspector already on the stack, else a free one.
      const onStack = s.layout.stack.find((id) => panes[id]?.kind === 'inspector')
      const id =
        onStack ??
        Object.values(panes).find(
          (p) => p.kind === 'inspector' && !s.layout.slotPanes.includes(p.id),
        )?.id ??
        nextId(panes, 'inspector')
      const prior = panes[id]
      panes[id] = {
        ...(prior ?? newPane(id, 'inspector')),
        target: ref,
        source: from,
        scrollTop: 0,
        history: nextHistory(prior),
      }
      const base = onStack
        ? s.layout.stack.slice(0, s.layout.stack.indexOf(onStack))
        : s.layout.stack
      return { panes, layout: { ...layout, stack: [...base, id] } }
    })
    const d = descriptorFor(ref.section)
    for (const cmd of d?.read?.(ref) ?? []) get().send(cmd)
    // US3 "not found" state (T050): the existing `list.status === 'ready' && !row` check in
    // Inspector.tsx only fires once the target's own section list has loaded. A related-resource
    // link can drill into a section whose list was never opened (e.g. Pod -> Node when the
    // Nodes list pane was never visited), so make sure it is: refreshSection is a no-op cost
    // relative to correctness here, and lists already tolerate redundant refreshes.
    if (push) get().refreshSection(ref.section)
  },

  goBack: (paneId) =>
    set((s) => {
      const pane = s.panes[paneId]
      const history = pane?.history
      if (!pane || !history || history.length === 0) return s
      const prevTarget = history[history.length - 1]
      return {
        panes: {
          ...s.panes,
          [paneId]: {
            ...pane,
            target: prevTarget,
            history: history.slice(0, -1),
            scrollTop: 0,
          },
        },
      }
    }),

  openEditSession: (ref, raw) =>
    set((s) => {
      const key = detailKey(s.session.activeContext, ref.kind, ref.namespace, ref.name)
      return { editSessions: { ...s.editSessions, [key]: openSession(ref, raw) } }
    }),

  openCreateEditSession: (target, initialYaml) => {
    const key = detailKey(get().session.activeContext, target.kind, target.namespace, target.name)
    set((s) => ({
      editSessions: { ...s.editSessions, [key]: openCreateSession(target, initialYaml) },
    }))
    return key
  },

  updateEditedYaml: (key, yaml) =>
    set((s) => {
      const session = s.editSessions[key]
      if (!session) return s
      return { editSessions: { ...s.editSessions, [key]: editYaml(session, yaml) } }
    }),

  reviewEditDiff: (key) =>
    set((s) => {
      const session = s.editSessions[key]
      if (!session) return s
      return { editSessions: { ...s.editSessions, [key]: reviewSessionDiff(session) } }
    }),

  discardEditSession: (key) =>
    set((s) => {
      const session = s.editSessions[key]
      if (!session) return s
      return { editSessions: { ...s.editSessions, [key]: discardSession(session) } }
    }),

  closeEditSession: (key) =>
    set((s) => {
      if (!(key in s.editSessions)) return s
      const editSessions = { ...s.editSessions }
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by detail key
      delete editSessions[key]
      return { editSessions }
    }),

  applyEditSession: async (key) => {
    const session = get().editSessions[key]
    if (!session) return
    const cmd = buildApplyCommand(session)
    let requestId: number
    try {
      requestId = await get().request(cmd)
    } catch (err) {
      set((s) => {
        const current = s.editSessions[key]
        if (!current) return s
        const message = err instanceof Error ? err.message : String(err)
        return { editSessions: { ...s.editSessions, [key]: markFailed(current, message) } }
      })
      return
    }
    set((s) => {
      const current = s.editSessions[key]
      if (!current) return s
      return { editSessions: { ...s.editSessions, [key]: markApplying(current, requestId) } }
    })
    const result = await waitForCommandResult(requestId)
    if (result === 'timeout') {
      set((s) => {
        const current = s.editSessions[key]
        if (!current || current.status !== 'applying') return s
        return {
          editSessions: {
            ...s.editSessions,
            [key]: markFailed(current, 'No response from the backend.'),
          },
        }
      })
      return
    }
    if (result.ok) return // the `ResourceApplied` stream event already marked it applied.
    set((s) => {
      const current = s.editSessions[key]
      if (!current) return s
      return {
        editSessions: {
          ...s.editSessions,
          [key]: markFailed(
            current,
            result.error ?? 'The backend reported a failure with no message.',
          ),
        },
      }
    })
  },

  toggleRowSelected: (paneId, rowKey) =>
    set((s) => {
      const current = s.selection[paneId] ?? []
      const next = current.includes(rowKey)
        ? current.filter((k) => k !== rowKey)
        : [...current, rowKey]
      return { selection: { ...s.selection, [paneId]: next } }
    }),

  setSelection: (paneId, rowKeys) =>
    set((s) => ({ selection: { ...s.selection, [paneId]: rowKeys } })),

  openAction: (prompt) => set({ actionPrompt: prompt }),

  closeAction: () => set({ actionPrompt: undefined }),

  setExpandedGroups: (groups) => {
    set((s) => ({ prefs: { ...s.prefs, expandedGroups: groups } }))
    scheduleUiSave(get)
  },

  closeInspector: () =>
    set((s) => {
      const panes = { ...s.panes }
      const clearSource = (id: PaneId | undefined) => {
        const src = id ? panes[id] : undefined
        if (src) panes[src.id] = { ...src, selection: undefined }
      }
      const top = s.layout.stack.at(-1)
      const topPane = top ? panes[top] : undefined
      // An inspector pushed onto `layout.stack` (phone always; tablet/desktop's "tool tab has no
      // dedicated inspector slot" fallback in `openInspector`) is closed by popping the stack.
      // An inspector living in a fixed row slot (`layout.slotPanes`, the desktop/wide grid) is
      // closed by clearing its target instead, in the loop below.
      if (topPane?.kind === 'inspector' && !s.layout.slotPanes.includes(topPane.id)) {
        clearSource(topPane.source)
        const stack = s.layout.stack.length > 1 ? s.layout.stack.slice(0, -1) : s.layout.stack
        return { panes, layout: { ...s.layout, stack } }
      }
      for (const id of s.layout.slotPanes) {
        const p = panes[id]
        if (p?.kind !== 'inspector' || !p.target) continue
        clearSource(p.source)
        panes[id] = { ...p, target: undefined, source: undefined }
      }
      return { panes }
    }),

  setNodeAction: (nodeName, action) =>
    set((s) => {
      const nodeActions = { ...s.nodeActions }
      if (action) {
        nodeActions[nodeName] = action
      } else {
        // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by node name
        delete nodeActions[nodeName]
      }
      return { nodeActions }
    }),

  setHelmUninstall: (key, state) =>
    set((s) => {
      const helmUninstalls = { ...s.helmUninstalls }
      if (state) {
        helmUninstalls[key] = state
      } else {
        // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by namespace/release
        delete helmUninstalls[key]
      }
      return { helmUninstalls }
    }),

  setHelmMutation: (key, session) =>
    set((s) => {
      const helmMutations = { ...s.helmMutations }
      if (session) {
        helmMutations[key] = session
      } else {
        // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by release
        delete helmMutations[key]
      }
      return { helmMutations }
    }),
}))

/** Detail cache lookup for the active context. */
export function selectDetail(
  s: StoreData,
  kind: string,
  namespace: string | null,
  name: string,
): DetailState | undefined {
  return s.details[detailKey(s.session.activeContext, kind, namespace, name)]
}

/** Cluster events whose involved object is `kind`/`namespace`/`name` (inspector Events tab). */
export function eventsFor(
  s: StoreData,
  kind: string,
  namespace: string,
  name: string,
): KubernetesEvent[] {
  const list = s.lists[listKey(s.session.activeContext, SECTION.events, 'Cluster', null)]
  return (list?.rows ?? []).filter(
    (e): e is KubernetesEvent =>
      typeof e === 'object' &&
      'involved_object_kind' in e &&
      e.involved_object_kind === kind &&
      e.involved_object_name === name &&
      e.namespace === namespace,
  )
}
