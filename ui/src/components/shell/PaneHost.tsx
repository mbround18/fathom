import { ArrowLineRightIcon, MagnifyingGlassIcon } from '@phosphor-icons/react'
import { useCallback, useLayoutEffect, useRef, type UIEvent } from 'react'

import { InspectorActions, ListActions, SelectionBar } from '@/components/ActionMenu'
import { Inspector } from '@/components/Inspector'
import { CertManagerPanel } from '@/components/panels/CertManagerPanel'
import OverviewPanel from '@/components/panels/OverviewPanel'
import EventsPanel from '@/components/panels/EventsPanel'
import MetricsPanel from '@/components/panels/MetricsPanel'
import GrafanaPanel from '@/components/panels/GrafanaPanel'
import LogsToolPanel from '@/components/panels/LogsToolPanel'
import ArgoPanel from '@/components/panels/ArgoPanel'
import { FluxPanel } from '@/components/panels/FluxPanel'
import { KnativePanel } from '@/components/panels/KnativePanel'
import { GatekeeperPanel } from '@/components/panels/GatekeeperPanel'
import { VeleroPanel } from '@/components/panels/VeleroPanel'
import { HubblePanel } from '@/components/panels/HubblePanel'
import { ComposeProjectsPane } from '@/features/containers/ComposeProjectsPane'
import { ContainersScene } from '@/features/containers/ContainersScene'
import { ObjectPane } from '@/features/containers/ObjectPanes'
import { DaemonTimelinePanel } from '@/features/daemon/DaemonTimelinePanel'
import { TrivyPanel } from '@/components/panels/TrivyPanel'
import { KedaPanel } from '@/components/panels/KedaPanel'
import { LinterPanel } from '@/components/panels/LinterPanel'
import { TopologyPanel } from '@/components/panels/TopologyPanel'
import { CustomResourcesSection } from '@/components/nav/CustomResourcesSection'
import { NavTree } from '@/components/NavTree'
import { ClusterSettingsPanel } from '@/components/panels/ClusterSettingsPanel'
import { AppSettingsPanel } from '@/components/settings/AppSettingsPanel'
import { LocalClustersPanel } from '@/components/settings/LocalClustersPanel'
import { AuditLogPane } from '@/components/audit/AuditLogPane'
import { PortForwardsPanel } from '@/components/panels/PortForwardsPanel'
import { TeamsView } from '@/features/teams/TeamsView'
import { TeamProfilesPanel } from '@/components/panels/TeamProfilesPanel'
import { RoleBindingCreateLauncher } from '@/components/rbac/RoleBindingCreateLauncher'
import { RoleAssignmentLauncher } from '@/features/roles/RoleAssignmentLauncher'
import { RoleBindingEditLauncher } from '@/components/rbac/RoleBindingEditLauncher'
import { RoleCreateLauncher } from '@/components/rbac/RoleCreateLauncher'
import { RoleEditLauncher } from '@/components/rbac/RoleEditLauncher'
import { ResourceTable } from '@/components/ResourceTable'
import { Input } from '@/components/ui/input'
import { SECTION } from '@/protocol/commands'
import type {
  ClusterRoleBindingDetails,
  ClusterRoleDetails,
  RoleBindingDetails,
  RoleDetails,
} from '@/protocol/types'
import { descriptorFor } from '@/resources'
import { NamespaceAccessView } from '@/resources/namespaceAccess'
import { OidcConfigView } from '@/resources/oidcConfig'
import { SignInSettings } from '@/features/identity/SignInSettings'
import { cn } from '@/lib/utils'
import { useFirstRun } from '@/lib/use-first-run'
import {
  deniedKey,
  detailKey,
  detailKeyOf,
  listKey,
  useStore,
  type PaneId,
  type PaneState,
} from '@/store'

import { CASCADE_FIRST_SLOT } from '@/layout/cascade'
import { paneDragProps, PaneSlotMenu } from '@/layout/PaneHeader'

import { cascadeWaitingTitle, paneTitle } from './labels'
import { FirstRunState } from './FirstRunState'
import {
  Disconnected,
  Empty,
  ErrorState,
  Loading,
  NoLongerExists,
  NotPermitted,
  StateFrame,
} from './PaneStates'

interface PaneHostProps {
  paneId: PaneId
  /** Position among visible slots; omitted for dock tabs. */
  slotIndex?: number
  className?: string
}

/**
 * Renders one pane by kind. All view state (search, selection, scroll) lives in
 * `store.panes[paneId]`, so the host can unmount on re-layout and come back intact (FR-007).
 */
export function PaneHost({ paneId, slotIndex, className }: PaneHostProps) {
  const pane = useStore((s) => s.panes[paneId])
  const updatePane = useStore((s) => s.updatePane)
  const focusList = useStore((s) => s.focusList)
  /** 044: the step the auto-open filled, marked until the operator clicks in the cascade. */
  const autoOpened = useStore((s) => s.layout.autoOpened)
  /**
   * 044 (T023): this column's cascade number, from 1 at slot 2 (contract "Slots and test ids").
   * Positional, so a chain that shifted left still reads 1, 2, 3 across the row (FR-006).
   */
  const stepNumber = useStore((s) =>
    slotIndex !== undefined &&
    slotIndex >= CASCADE_FIRST_SLOT &&
    (s.layout.widthClass === 'wide' || s.layout.widthClass === 'ultra')
      ? String(slotIndex - CASCADE_FIRST_SLOT + 1)
      : undefined,
  )
  const scrollRef = useRef<HTMLDivElement>(null)
  const frame = useRef(0)

  // Restore scroll before paint; read once so scroll writes do not feed back.
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = useStore.getState().panes[paneId]?.scrollTop ?? 0
    return () => cancelAnimationFrame(frame.current)
  }, [paneId])

  const onScroll = useCallback(
    (e: UIEvent<HTMLDivElement>) => {
      const top = e.currentTarget.scrollTop
      cancelAnimationFrame(frame.current)
      frame.current = requestAnimationFrame(() => updatePane(paneId, { scrollTop: top }))
    },
    [paneId, updatePane],
  )

  if (!pane) return null
  /** An occupied cascade column; an empty one carries `cascade-empty` from `CascadeInspector`. */
  const step = pane.kind === 'inspector' && pane.target !== undefined ? stepNumber : undefined
  /** A cascade column with nothing in it yet (T052a): its header says what it is reserved for. */
  const waiting = pane.kind === 'inspector' && pane.target === undefined ? stepNumber : undefined
  const title = waiting !== undefined ? cascadeWaitingTitle(waiting) : paneTitle(pane)
  // Lists and the inspector scroll inside (virtualized body, tab panels); the rest scroll here.
  const selfScrolling = pane.kind === 'list' || pane.kind === 'inspector'
  const onFocusWithin = pane.kind === 'list' ? () => focusList(pane.id) : undefined
  /**
   * 044: a click anywhere in the cascade is the operator taking over — it drops the auto-open
   * marker and any arm that has not fired yet, so a slow list cannot steal their selection.
   * Read imperatively (no subscription) and only dispatched when there is something to drop.
   */
  const onPointerDown =
    slotIndex === undefined
      ? onFocusWithin
      : () => {
          onFocusWithin?.()
          const { layout, clearAutoOpen } = useStore.getState()
          if (layout.pendingAutoOpen !== undefined || layout.autoOpened !== undefined) {
            clearAutoOpen()
          }
        }

  return (
    <section
      aria-label={title}
      data-slot="pane"
      data-slot-index={slotIndex}
      data-pane-kind={pane.kind}
      data-pane-id={pane.id}
      data-section={pane.route?.section}
      data-tool={pane.kind === 'tool' ? pane.tool : undefined}
      data-auto-open={autoOpened === pane.id ? 'true' : undefined}
      data-testid={step !== undefined ? 'cascade-step' : undefined}
      data-step={step}
      onPointerDownCapture={onPointerDown}
      onFocusCapture={onFocusWithin}
      className={cn('flex h-full min-h-0 min-w-0 flex-col bg-background', className)}
    >
      <header
        className="flex h-[max(2rem,var(--touch-target))] shrink-0 items-center gap-2 border-b border-border bg-card px-2"
        {...(pane.kind === 'list' && slotIndex !== undefined ? paneDragProps(pane.id) : {})}
      >
        {slotIndex !== undefined && (
          <span aria-hidden="true" className="text-xs text-accent-neon">
            {String(slotIndex + 1).padStart(2, '0')}
          </span>
        )}
        <h2
          className={cn(
            'min-w-0 truncate text-xs font-semibold',
            pane.kind === 'inspector' && pane.target
              ? 'font-mono text-accent-neon'
              : 'font-display tracking-wider uppercase',
          )}
        >
          {title}
        </h2>
        <span className="ml-auto" />
        {pane.kind === 'list' && slotIndex !== undefined && <PaneSlotMenu paneId={pane.id} />}
        {pane.kind === 'list' && <ListActions pane={pane} />}
        {pane.kind === 'inspector' && pane.target && (
          <InspectorActions
            target={pane.target}
            {...(pane.source !== undefined ? { source: pane.source } : {})}
          />
        )}
        {pane.kind === 'inspector' && pane.target && (
          <RoleBindingEditLauncherSlot target={pane.target} />
        )}
        {pane.kind === 'inspector' && pane.target && <RoleEditLauncherSlot target={pane.target} />}
      </header>
      {pane.kind === 'list' && <PaneSearch pane={pane} />}
      {selfScrolling ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <PaneBody pane={pane} />
        </div>
      ) : (
        <div ref={scrollRef} onScroll={onScroll} className="min-h-0 flex-1 overflow-auto">
          <PaneBody pane={pane} />
        </div>
      )}
    </section>
  )
}

/**
 * 011-rbac-oidc-administration US3 (T032): the "Edit" entry point for an already-open
 * RoleBinding/ClusterRoleBinding inspector, seeded from the same detail payload the inspector
 * itself renders (`s.details[detailKey(...)]`) — no extra read issued here.
 */
function RoleBindingEditLauncherSlot({ target }: { target: NonNullable<PaneState['target']> }) {
  const ctx = useStore((s) => s.session.activeContext)
  const detail = useStore(
    (s) => s.details[detailKey(ctx, target.kind, target.namespace, target.name)],
  )
  if (target.kind === 'RoleBinding' && detail?.eventType === 'RoleBindingDetailsUpdated') {
    return (
      <RoleBindingEditLauncher kind="RoleBinding" existing={detail.data as RoleBindingDetails} />
    )
  }
  if (
    target.kind === 'ClusterRoleBinding' &&
    detail?.eventType === 'ClusterRoleBindingDetailsUpdated'
  ) {
    return (
      <RoleBindingEditLauncher
        kind="ClusterRoleBinding"
        existing={detail.data as ClusterRoleBindingDetails}
      />
    )
  }
  return null
}

/** 042-role-create: the "Edit" entry point for an open Role/ClusterRole inspector. */
function RoleEditLauncherSlot({ target }: { target: NonNullable<PaneState['target']> }) {
  const ctx = useStore((s) => s.session.activeContext)
  const detail = useStore(
    (s) => s.details[detailKey(ctx, target.kind, target.namespace, target.name)],
  )
  if (target.kind === 'Role' && detail?.eventType === 'RoleDetailsUpdated') {
    return <RoleEditLauncher kind="Role" existing={detail.data as RoleDetails} />
  }
  if (target.kind === 'ClusterRole' && detail?.eventType === 'ClusterRoleDetailsUpdated') {
    return <RoleEditLauncher kind="ClusterRole" existing={detail.data as ClusterRoleDetails} />
  }
  return null
}

function PaneSearch({ pane }: { pane: PaneState }) {
  const updatePane = useStore((s) => s.updatePane)
  return (
    <div className="relative shrink-0 border-b border-border">
      <MagnifyingGlassIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
      />
      <Input
        type="search"
        aria-label={`Filter ${paneTitle(pane)}`}
        placeholder="filter…"
        value={pane.search}
        onChange={(e) => updatePane(pane.id, { search: e.target.value })}
        className="h-[var(--touch-target)] rounded-none border-0 pl-7 focus-visible:ring-inset"
      />
    </div>
  )
}

function PaneBody({ pane }: { pane: PaneState }) {
  switch (pane.kind) {
    case 'nav':
      return <NavTree />
    case 'list':
      return <ListBody pane={pane} />
    case 'inspector':
      return <CascadeInspector pane={pane} />
    case 'tool':
      if (pane.tool === 'metrics') return <MetricsPanel />
      if (pane.tool === 'flux') return <FluxPanel />
      if (pane.tool === 'knative') return <KnativePanel />
      if (pane.tool === 'velero') return <VeleroPanel />
      if (pane.tool === 'certManager') return <CertManagerPanel />
      if (pane.tool === 'hubble') return <HubblePanel />
      if (pane.tool === 'vulnerabilities') return <TrivyPanel />
      if (pane.tool === 'keda') return <KedaPanel />
      if (pane.tool === 'topology') return <TopologyPanel paneId={pane.id} />
      if (pane.tool === 'daemon') return <DaemonTimelinePanel />
      if (pane.tool === 'linter') return <LinterPanel paneId={pane.id} />
      if (pane.tool === 'portforwards') return <PortForwardsPanel />
      if (pane.tool === 'events') return <EventsPanel />
      if (pane.tool === 'overview') return <OverviewPanel />
      if (pane.tool === 'logs') return <LogsToolPanel />
      return (
        <Empty title="Tool" detail="No data yet. Connect to a cluster to populate this view." />
      )
  }
}

/**
 * 044 (FR-002/FR-012/FR-014): at wide and above an inspector is a cascade column. With nothing
 * open it says what the next click will put there instead of reading as a broken layout; holding
 * an object the cluster no longer has, it says so and keeps its target, so nothing else is
 * silently opened in its place. Below wide the inspector keeps its own states, so small windows
 * are untouched (FR-009).
 */
function CascadeInspector({ pane }: { pane: PaneState }) {
  const cascading = useStore(
    (s) => s.layout.widthClass === 'wide' || s.layout.widthClass === 'ultra',
  )
  const gone = useCascadeGone(pane.target)
  if (!cascading) return <Inspector pane={pane} />
  if (!pane.target) return <CascadeEmpty />
  if (gone) return <CascadeGone target={pane.target} />
  return <Inspector pane={pane} />
}

/**
 * FR-012: a column that is waiting, not broken. The dashed frame marks it as reserved and the
 * arrow points at where the next level lands; the copy is the contract's
 * (`contracts/ui-cascade.md` § Slots and test ids). `StateFrame` carries the announcement
 * (`role="status"`, polite) and the app's AA-checked text tokens (`tests/unit/contrast.test.ts`),
 * so this adds no colour of its own. Only `h-full`: the pane's own size constraints are untouched,
 * so filling this column reflows nothing (FR-013).
 */
function CascadeEmpty() {
  return (
    <div data-testid="cascade-empty" className="h-full p-2">
      <div className="h-full rounded-sm border border-dashed border-border">
        <StateFrame
          kind="cascade-empty"
          icon={ArrowLineRightIcon}
          title="Waiting"
          detail="Follow a link to open the next level here"
          trace
        />
      </div>
    </div>
  )
}

/**
 * FR-014: the step's object has gone from the list or the cluster. The column keeps `pane.target`
 * (the header still names it) so the cascade never silently re-points at a different object; the
 * operator's next click is what moves it.
 */
function CascadeGone({ target }: { target: NonNullable<PaneState['target']> }) {
  const label = target.namespace ? `${target.namespace}/${target.name}` : target.name
  return (
    <div data-testid="cascade-gone" className="h-full">
      <NoLongerExists name={label} />
    </div>
  )
}

/**
 * FR-014: gone means the object's own section list has refreshed without it, or its read came back
 * not-found. Mirrors the checks in `Inspector.tsx`, which still serve every width below wide.
 */
function useCascadeGone(target: PaneState['target']): boolean {
  const ctx = useStore((s) => s.session.activeContext)
  const descriptor = target ? descriptorFor(target.section) : undefined
  const scope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const list = useStore((s) =>
    target === undefined ? undefined : s.lists[listKey(ctx, target.section, scope, null)],
  )
  const detail = useStore((s) =>
    target === undefined ? undefined : s.details[detailKeyOf(ctx, target)],
  )
  if (!target || !descriptor) return false
  if (detail?.eventType === 'FluxResourceNotFound') return true
  const rowKey = target.namespace ? `${target.namespace}/${target.name}` : target.name
  return list?.status === 'ready' && !list.rows.some((r) => descriptor.rowKey(r) === rowKey)
}

/** An API 404 on a list means the resource type (a CRD) is not installed, not that it failed. */
function isNotInstalledError(message: string | undefined): boolean {
  return (
    message !== undefined &&
    /\b404\b|not\s?found|could not find the requested resource/i.test(message)
  )
}

/** 031: the section whose denial a pane shares (Argo reads the CRD catalog). */
function deniedVia(section: string): string {
  return section === SECTION.argo ? SECTION.customResources : section
}

/** 043 US2: Settings › Sign-in. */
const SIGN_IN_SECTION = 'settings/sign-in'

/** Sections that render without a connected cluster (local settings and records). */
export const CLUSTERLESS_SECTIONS: ReadonlySet<string> = new Set([
  SECTION.clusterSettings,
  SECTION.teams,
  SECTION.teamProfiles,
  SECTION.auditLog,
  SECTION.oidcConfig,
  SECTION.appSettings,
  SECTION.localClusters,
  SIGN_IN_SECTION,
  // 048: this computer's runtime has nothing to do with a cluster being connected.
  SECTION.containers,
  SECTION.containerProjects,
  SECTION.containerImages,
  SECTION.containerVolumes,
  SECTION.containerNetworks,
])

/** A list pane: one clear state (FR-015), else the table with a strip for refresh/errors. */
function ListBody({ pane }: { pane: PaneState }) {
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const firstRun = useFirstRun()
  const ctx = useStore((s) => s.session.activeContext)
  const section = pane.route?.section
  const descriptor = descriptorFor(section)
  const scope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const list = useStore((s) =>
    section === undefined ? undefined : s.lists[listKey(ctx, section, scope, null)],
  )
  const refreshSection = useStore((s) => s.refreshSection)
  const denied = useStore((s) =>
    section === undefined ? undefined : s.denied[deniedKey(ctx, deniedVia(section))],
  )

  if (disconnected) return <Disconnected />
  if (section === undefined) {
    return <Empty title="Pick a section" detail="Choose a resource type from the navigator." />
  }
  // First run / cluster unusable: explain it with a next step instead of a list that can only
  // fail. Sections that work without a cluster (settings, audit log, generators) still open.
  if (firstRun && !CLUSTERLESS_SECTIONS.has(section)) return <FirstRunState state={firstRun} />
  // 031: a section the identity may not read (RBAC 403) says so, whatever renders it; the
  // Overview instead degrades card by card.
  if (denied !== undefined && section !== SECTION.overview && !(list?.rows.length ?? 0)) {
    return <NotPermitted what={descriptor?.label.toLowerCase()} />
  }
  // US3: generic CRD browsing has its own catalog/instance/detail layout instead of the
  // built-in-kind descriptor registry (which is a closed, compile-time set of kinds).
  if (section === SECTION.customResources) {
    return <CustomResourcesSection />
  }
  // 009 US2: the forwards list is a computed merge (PortForward + tunnel status + origin tag),
  // not a plain descriptor table, so it renders itself directly here (desktop nav entry point;
  // phone/tablet's `FORWARDS_TAB` reaches the same component via the `'tool'` pane branch above).
  if (section === SECTION.portForwards) {
    return <PortForwardsPanel />
  }
  // 014-teams-organizations: local §10 Cluster Settings (+ org sync) and shared connection
  // profiles are settings forms, not resource tables — same "render itself directly" pattern
  // as `portForwards` above, no descriptor.
  // 020-gatekeeper-policy-pane: read-only constraint compliance view, no descriptor.
  if (section === SECTION.gatekeeper) {
    return <GatekeeperPanel />
  }
  if (section === SECTION.clusterSettings) {
    return <ClusterSettingsPanel />
  }
  if (section === SECTION.teams) {
    return <TeamsView />
  }
  if (section === SECTION.auditLog) {
    return <AuditLogPane />
  }
  if (section === SECTION.teamProfiles) {
    return <TeamProfilesPanel />
  }
  // 043 US3/US4: settings split out of Cluster settings, and the daemon's Cluster history.
  if (section === SECTION.appSettings) return <AppSettingsPanel />
  if (section === SECTION.localClusters) return <LocalClustersPanel />
  // 048-docker-management US1/US3.
  if (section === SECTION.containers) return <ContainersScene />
  if (section === SECTION.containerProjects) return <ComposeProjectsPane />
  if (section === SECTION.containerImages) return <ObjectPane objectClass="image" />
  if (section === SECTION.containerVolumes) return <ObjectPane objectClass="volume" />
  if (section === SECTION.containerNetworks) return <ObjectPane objectClass="network" />
  if (section === SECTION.clusterHistory) return <DaemonTimelinePanel />
  // 011-rbac-oidc-administration US4/US5: read-only lookup / frontend-only generator panels,
  // neither backed by a `ResourceDescriptor` (no list-refresh, no detail split).
  if (section === SECTION.namespaceAccess) {
    return <NamespaceAccessView />
  }
  if (section === SECTION.oidcConfig) {
    return <OidcConfigView />
  }
  if (section === SIGN_IN_SECTION) {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        <SignInSettings />
      </div>
    )
  }
  // 021-trivy-vulnerability-pane: cluster-wide summary, no descriptor (renders itself).
  if (section === SECTION.vulnerabilities) {
    return <TrivyPanel />
  }
  // 043 US6 (FR-030): the whole-cluster topology is a navigator section in the Cluster group.
  if (section === 'topology') return <TopologyPanel paneId={pane.id} />
  if (section === SECTION.overview) return <OverviewPanel paneId={pane.id} />
  // 043: the linter left the dock for Cluster › Linter (same panel, same visuals).
  if (section === 'linter') return <LinterPanel paneId={pane.id} />
  if (section === SECTION.events) return <EventsPanel />
  if (section === SECTION.metrics) return <MetricsPanel />
  if (section === SECTION.grafana) return <GrafanaPanel />
  if (section === SECTION.argo) return <ArgoPanel />
  if (section === SECTION.flux) return <FluxPanel />
  if (section === SECTION.knative) return <KnativePanel />
  if (section === SECTION.keda) return <KedaPanel />
  if (section === SECTION.velero) return <VeleroPanel />
  if (section === SECTION.certManager) return <CertManagerPanel />
  if (section === SECTION.hubble) return <HubblePanel />
  if (!descriptor) {
    return (
      <Empty
        title="Section unavailable"
        detail="This section has no view in this version of Fathom."
      />
    )
  }
  const hasRows = (list?.rows.length ?? 0) > 0
  if (!list || (list.status === 'loading' && !hasRows) || (list.status === 'idle' && !hasRows)) {
    return <Loading label={`Loading ${descriptor.label}`} />
  }
  if (list.status === 'error' && !hasRows) {
    if (list.forbidden) return <NotPermitted what={descriptor.label.toLowerCase()} />
    if (isNotInstalledError(list.error)) {
      return (
        <Empty
          title={`${descriptor.label} not available`}
          detail="This resource type is not installed on the cluster (its CRD was not found)."
        />
      )
    }
    return (
      <ErrorState
        title={`Cannot list ${descriptor.label.toLowerCase()}`}
        detail={list.error}
        onRetry={() => refreshSection(section)}
      />
    )
  }
  return (
    <>
      {list.status === 'loading' && (
        <div
          role="status"
          aria-label="Refreshing"
          className="h-0.5 shrink-0 animate-pulse bg-accent-neon/70"
        />
      )}
      {list.status === 'error' && (
        <p
          role="alert"
          className="shrink-0 border-b border-status-err/50 px-2 py-1 text-xs text-status-err"
        >
          <span aria-hidden="true">✖ </span>
          {list.error}
        </p>
      )}
      {/* 042-role-create: "New Role"/"New ClusterRole". */}
      {section === SECTION.roles && (
        <div className="flex shrink-0 justify-end gap-2 border-b border-border px-2 py-1">
          <RoleCreateLauncher kind="Role" acceptRequests />
        </div>
      )}
      {section === SECTION.clusterRoles && (
        <div className="flex shrink-0 justify-end gap-2 border-b border-border px-2 py-1">
          <RoleCreateLauncher kind="ClusterRole" acceptRequests />
        </div>
      )}
      {/* 011-rbac-oidc-administration US2 (T029): "New RoleBinding"/"New ClusterRoleBinding".
          043 US8: the 042 role wizard next to them (a ClusterRoleBinding can only use a
          ClusterRole, so its toolbar offers only that). */}
      {section === SECTION.roleBindings && (
        <div className="flex shrink-0 flex-wrap justify-end gap-2 border-b border-border px-2 py-1">
          <RoleCreateLauncher
            kind="Role"
            label="New Role…"
            testId="rbac-bindings-new-role"
            variant="outline"
          />
          <RoleCreateLauncher
            kind="ClusterRole"
            label="New ClusterRole…"
            testId="rbac-bindings-new-clusterrole"
            variant="outline"
          />
          <RoleBindingCreateLauncher kind="RoleBinding" />
          <RoleAssignmentLauncher kind="RoleBinding" />
        </div>
      )}
      {section === SECTION.clusterRoleBindings && (
        <div className="flex shrink-0 flex-wrap justify-end gap-2 border-b border-border px-2 py-1">
          <RoleCreateLauncher
            kind="ClusterRole"
            label="New ClusterRole…"
            testId="rbac-bindings-new-clusterrole"
            variant="outline"
          />
          <RoleBindingCreateLauncher kind="ClusterRoleBinding" />
          <RoleAssignmentLauncher kind="ClusterRoleBinding" />
        </div>
      )}
      <SelectionBar pane={pane} descriptor={descriptor} rows={list.rows} />
      <div className="min-h-0 flex-1">
        <ResourceTable pane={pane} descriptor={descriptor} rows={list.rows} />
      </div>
    </>
  )
}
