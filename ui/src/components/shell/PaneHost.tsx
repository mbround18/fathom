import { MagnifyingGlassIcon } from '@phosphor-icons/react'
import { useCallback, useLayoutEffect, useRef, type UIEvent } from 'react'

import { InspectorActions, ListActions, SelectionBar } from '@/components/ActionMenu'
import { Inspector } from '@/components/Inspector'
import { CertManagerPanel } from '@/components/panels/CertManagerPanel'
import OverviewPanel from '@/components/panels/OverviewPanel'
import EventsPanel from '@/components/panels/EventsPanel'
import MetricsPanel from '@/components/panels/MetricsPanel'
import GrafanaPanel from '@/components/panels/GrafanaPanel'
import ArgoPanel from '@/components/panels/ArgoPanel'
import { FluxPanel } from '@/components/panels/FluxPanel'
import { KnativePanel } from '@/components/panels/KnativePanel'
import { GatekeeperPanel } from '@/components/panels/GatekeeperPanel'
import { VeleroPanel } from '@/components/panels/VeleroPanel'
import { HubblePanel } from '@/components/panels/HubblePanel'
import { DaemonTimelinePanel } from '@/features/daemon/DaemonTimelinePanel'
import { TrivyPanel } from '@/components/panels/TrivyPanel'
import { KedaPanel } from '@/components/panels/KedaPanel'
import { LinterPanel } from '@/components/panels/LinterPanel'
import { TopologyPanel } from '@/components/panels/TopologyPanel'
import { UnavailableSection } from '@/components/panels/UnavailableSection'
import { CustomResourcesSection } from '@/components/nav/CustomResourcesSection'
import { NavTree } from '@/components/NavTree'
import { ClusterSettingsPanel } from '@/components/panels/ClusterSettingsPanel'
import { AuditLogPane } from '@/components/audit/AuditLogPane'
import { PortForwardsPanel } from '@/components/panels/PortForwardsPanel'
import { TeamsView } from '@/features/teams/TeamsView'
import { TeamProfilesPanel } from '@/components/panels/TeamProfilesPanel'
import { RoleBindingCreateLauncher } from '@/components/rbac/RoleBindingCreateLauncher'
import { RoleAssignmentLauncher } from '@/features/roles/RoleAssignmentLauncher'
import { RoleBindingEditLauncher } from '@/components/rbac/RoleBindingEditLauncher'
import { ResourceTable } from '@/components/ResourceTable'
import { Input } from '@/components/ui/input'
import { SECTION } from '@/protocol/commands'
import type { ClusterRoleBindingDetails, RoleBindingDetails } from '@/protocol/types'
import { descriptorFor } from '@/resources'
import { NamespaceAccessView } from '@/resources/namespaceAccess'
import { OidcConfigView } from '@/resources/oidcConfig'
import { cn } from '@/lib/utils'
import { detailKey, listKey, useStore, type PaneId, type PaneState } from '@/store'

import { paneTitle, TOOL_LABEL } from './labels'
import {
  Disconnected,
  Empty,
  ErrorState,
  Loading,
  NoKubeconfig,
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
  const title = paneTitle(pane)
  // Lists and the inspector scroll inside (virtualized body, tab panels); the rest scroll here.
  const selfScrolling = pane.kind === 'list' || pane.kind === 'inspector'
  const onFocusWithin = pane.kind === 'list' ? () => focusList(pane.id) : undefined

  return (
    <section
      aria-label={title}
      data-slot="pane"
      data-slot-index={slotIndex}
      data-pane-kind={pane.kind}
      data-pane-id={pane.id}
      data-section={pane.route?.section}
      onPointerDownCapture={onFocusWithin}
      onFocusCapture={onFocusWithin}
      className={cn('flex h-full min-h-0 min-w-0 flex-col bg-background', className)}
    >
      <header className="flex h-[max(2rem,var(--touch-target))] shrink-0 items-center gap-2 border-b border-border bg-card px-2">
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
      return <Inspector pane={pane} />
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
      return (
        <Empty
          title={pane.tool ? TOOL_LABEL[pane.tool] : 'Tool'}
          detail={
            pane.tool === 'logs'
              ? 'Select a pod to stream logs.'
              : 'No data yet. Connect to a cluster to populate this view.'
          }
        />
      )
  }
}

/** An API 404 on a list means the resource type (a CRD) is not installed, not that it failed. */
function isNotInstalledError(message: string | undefined): boolean {
  return (
    message !== undefined &&
    /\b404\b|not\s?found|could not find the requested resource/i.test(message)
  )
}

/** A list pane: one clear state (FR-015), else the table with a strip for refresh/errors. */
function ListBody({ pane }: { pane: PaneState }) {
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const noKubeconfig = useStore((s) => s.session.contextsLoaded && s.session.contexts.length === 0)
  const notice = useStore((s) => s.conn.lastNotice?.message)
  const ctx = useStore((s) => s.session.activeContext)
  const section = pane.route?.section
  const descriptor = descriptorFor(section)
  const scope = descriptor?.namespaced === false ? 'Cluster' : 'Namespaced'
  const list = useStore((s) =>
    section === undefined ? undefined : s.lists[listKey(ctx, section, scope, null)],
  )
  const refreshSection = useStore((s) => s.refreshSection)

  if (disconnected) return <Disconnected />
  if (noKubeconfig) return <NoKubeconfig detail={notice} />
  if (section === undefined) {
    return <Empty title="Pick a section" detail="Choose a resource type from the navigator." />
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
  // 011-rbac-oidc-administration US4/US5: read-only lookup / frontend-only generator panels,
  // neither backed by a `ResourceDescriptor` (no list-refresh, no detail split).
  if (section === SECTION.namespaceAccess) {
    return <NamespaceAccessView />
  }
  if (section === SECTION.oidcConfig) {
    return <OidcConfigView />
  }
  // 021-trivy-vulnerability-pane: cluster-wide summary, no descriptor (renders itself).
  if (section === 'integrations/vulnerabilities') {
    return <TrivyPanel />
  }
  if (section === SECTION.overview) return <OverviewPanel />
  if (section === SECTION.events) return <EventsPanel />
  if (section === SECTION.metrics) return <MetricsPanel />
  if (section === SECTION.grafana) return <GrafanaPanel />
  if (section === SECTION.argo) return <ArgoPanel />
  if (section === SECTION.flux) return <FluxPanel />
  if (section === SECTION.verticalPodAutoscalers) {
    return <UnavailableSection label="Vertical Pod Autoscalers" />
  }
  if (!descriptor) {
    return (
      <Empty
        title="Not ported yet"
        detail="This section still lives in the egui app; it arrives in a later milestone."
      />
    )
  }
  const hasRows = (list?.rows.length ?? 0) > 0
  if (!list || (list.status === 'loading' && !hasRows) || (list.status === 'idle' && !hasRows)) {
    return <Loading label={`Loading ${descriptor.label}`} />
  }
  if (list.status === 'error' && !hasRows) {
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
      {/* 011-rbac-oidc-administration US2 (T029): "New RoleBinding"/"New ClusterRoleBinding". */}
      {section === SECTION.roleBindings && (
        <div className="flex shrink-0 justify-end gap-2 border-b border-border px-2 py-1">
          <RoleBindingCreateLauncher kind="RoleBinding" />
          <RoleAssignmentLauncher kind="RoleBinding" />
        </div>
      )}
      {section === SECTION.clusterRoleBindings && (
        <div className="flex shrink-0 justify-end gap-2 border-b border-border px-2 py-1">
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
