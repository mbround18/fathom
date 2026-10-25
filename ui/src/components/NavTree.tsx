/**
 * Section navigator (T065, 043 US5/US7): the `NAV_MODEL` groups in plain language, each entry
 * with a one-line description (tooltip and accessible description), rarely used kinds under a
 * per-group "More" toggle, and integrations greyed when not installed (Velero hidden until
 * detected). Used by the nav slot (laptop+) and the drawer (phone/tablet). Expanded groups and
 * "More" toggles persist through `SaveUiConfig` (`expanded_groups`, `"<Group>/More"`); labels
 * saved before 043 are mapped on read and rewritten only on the next change (FR-023).
 */
import { CaretRightIcon } from '@phosphor-icons/react'
import { useEffect, useMemo } from 'react'

import type { SectionKey } from '@/protocol/types'
import { cn } from '@companion/theme'
import { descriptorFor } from '@/resources'
import { integrationInstalled, NOT_INSTALLED_HINT } from '@/resources/integrationStatus'
import {
  DEFAULT_EXPANDED_GROUPS,
  entryVisible,
  mapLegacyGroups,
  moreKey,
  NAV_GROUPS,
  NAV_TOOLS,
  navEntry,
  PANEL_SECTIONS,
  type NavSectionGroup,
} from '@/resources/nav'
import { useNavVisibility } from '@/resources/useNavVisibility'
import { useStore } from '@/store'

import { ALL_TOOL_TABS } from './shell/stack-tabs'
import { useOpenTool } from './shell/useOpenTool'

const slug = (label: string) => label.replace(/\s+/g, '-').toLowerCase()

export function NavTree() {
  const navigate = useStore((s) => s.navigate)
  const openTool = useOpenTool()
  const setExpandedGroups = useStore((s) => s.setExpandedGroups)
  const saved = useStore((s) =>
    s.prefs.loaded ? s.prefs.expandedGroups : (DEFAULT_EXPANDED_GROUPS as string[]),
  )
  const expanded = useMemo(() => mapLegacyGroups(saved), [saved])
  const active = useStore((s) => {
    const id = s.layout.focusedList ?? s.layout.slotPanes.find((p) => s.panes[p]?.kind === 'list')
    return id ? s.panes[id]?.route?.section : undefined
  })

  const visibility = useNavVisibility()
  // 048-docker-management: the container-runtime probe is not sent from here. It belongs to the
  // session, not to this component — see `connectBackend` in `src/protocol/use-backend.ts`.
  const groups = useMemo<NavSectionGroup[]>(
    () =>
      NAV_GROUPS.map((g) => ({
        ...g,
        sections: g.sections.filter((x) => {
          const model = navEntry(x.key)
          return model === undefined || entryVisible(model, visibility)
        }),
      })).filter((g) => g.sections.length > 0),
    [visibility],
  )
  const integrationsOpen = expanded.includes('Integrations')
  const ctx = useStore((s) => s.session.activeContext)
  // Discover which optional integrations exist as soon as their group is visible, instead of
  // making the user open each pane first. Only ask about those never scanned (unknown).
  useEffect(() => {
    if (!integrationsOpen || ctx === undefined) return
    const st = useStore.getState()
    const probes: [SectionKey, () => void][] = [
      ['integrations/flux', st.refreshFlux],
      ['integrations/knative', st.refreshKnative],
      ['integrations/keda', st.refreshKeda],
      ['integrations/velero', st.refreshVelero],
      ['integrations/cert-manager', st.refreshCertManager],
      ['integrations/hubble', st.refreshHubble],
      ['integrations/gatekeeper', st.refreshGatekeeper],
      ['integrations/vulnerabilities', st.refreshTrivyOverview],
      ['integrations/argo', () => st.send({ type: 'RefreshCrds' })],
      // One metrics-stack scan answers both Metrics and Grafana.
      ['integrations/metrics', () => st.send({ type: 'RefreshIntegrations' })],
    ]
    for (const [key, refresh] of probes) if (integrationInstalled(st, key) === undefined) refresh()
  }, [integrationsOpen, ctx])

  const toggle = (key: string) =>
    setExpandedGroups(
      expanded.includes(key) ? expanded.filter((g) => g !== key) : [...expanded, key],
    )

  const entryButton = (s: NavSectionGroup['sections'][number]) => {
    const current = active === s.key
    const ported = descriptorFor(s.key) !== undefined || PANEL_SECTIONS.has(s.key)
    const absent = visibility.installed(s.key) === false
    const descId = `nav-desc-${slug(s.key)}`
    return (
      <li key={s.key} className="flex">
        <button
          type="button"
          aria-current={current ? 'page' : undefined}
          aria-describedby={absent ? `${descId}-hint ${descId}` : descId}
          data-section={s.key}
          data-installed={absent ? 'false' : undefined}
          title={absent ? `${NOT_INSTALLED_HINT}. ${s.description}` : s.description}
          onClick={() => navigate(s.key)}
          className={cn(
            'relative flex h-[var(--row-height)] w-full min-w-0 items-center pr-3 pl-7 text-left outline-none',
            'hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
            (!ported || absent) && !current && 'text-muted-foreground',
            current && 'bg-primary text-primary-foreground hover:bg-primary',
          )}
        >
          <span className="truncate">{s.label}</span>
        </button>
        <span id={descId} className="sr-only">
          {s.description}
        </span>
        {absent && (
          <span id={`${descId}-hint`} className="sr-only">
            {NOT_INSTALLED_HINT}
          </span>
        )}
      </li>
    )
  }

  return (
    <nav aria-label="Sections" className="flex flex-col py-1">
      {groups.map((group) => {
        const open = expanded.includes(group.label)
        const id = `nav-group-${slug(group.label)}`
        const basic = group.sections.filter((x) => !x.advanced)
        const advanced = group.sections.filter((x) => x.advanced)
        const more = moreKey(group.label)
        const moreOpen = expanded.includes(more)
        return (
          <div key={group.label} className="flex flex-col">
            <button
              type="button"
              aria-expanded={open}
              aria-controls={id}
              onClick={() => toggle(group.label)}
              className={cn(
                'group/g flex h-[var(--row-height)] shrink-0 items-center gap-2 px-2 text-left outline-none',
                'font-display text-xs font-semibold tracking-[0.04em] uppercase',
                'text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
              )}
            >
              <CaretRightIcon
                aria-hidden="true"
                className={cn('size-3 shrink-0 transition-transform', open && 'rotate-90')}
              />
              <span className="min-w-0 flex-1 truncate">{group.label}</span>
              <span
                aria-hidden="true"
                className="font-brand text-xs tracking-normal text-accent-neon group-hover/g:text-accent-neon"
              >
                {group.kanji}
              </span>
            </button>
            {open && (
              <ul id={id} className="flex flex-col pb-1">
                {basic.map(entryButton)}
                {advanced.length > 0 && (
                  <li className="flex flex-col">
                    <button
                      type="button"
                      aria-label={`More ${group.label}`}
                      aria-expanded={moreOpen}
                      aria-controls={`${id}-more`}
                      data-more={group.label}
                      onClick={() => toggle(more)}
                      className={cn(
                        'flex h-[var(--row-height)] w-full min-w-0 items-center gap-1.5 pr-3 pl-7 text-left text-xs text-muted-foreground outline-none',
                        'hover:bg-muted hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset',
                      )}
                    >
                      <CaretRightIcon
                        aria-hidden="true"
                        className={cn(
                          'size-3 shrink-0 transition-transform',
                          moreOpen && 'rotate-90',
                        )}
                      />
                      <span className="truncate">More ({advanced.length})</span>
                    </button>
                    {moreOpen && (
                      <ul id={`${id}-more`} className="flex flex-col">
                        {advanced.map(entryButton)}
                      </ul>
                    )}
                  </li>
                )}
              </ul>
            )}
          </div>
        )
      })}
      <div className="flex flex-col">
        <p className="flex h-[var(--row-height)] shrink-0 items-center gap-2 px-2 pl-7 font-display text-xs font-semibold tracking-[0.12em] text-muted-foreground uppercase">
          Tools
        </p>
        <ul className="flex flex-col pb-1">
          {[
            { label: 'Fleet', tool: 'fleet' as const },
            ...ALL_TOOL_TABS.filter((t) => t.tool && NAV_TOOLS.includes(t.tool)),
          ].map((t) => (
            <li key={t.label} className="flex">
              <button
                type="button"
                data-tool={t.tool}
                onClick={() => t.tool && openTool(t.tool)}
                className="flex h-[var(--row-height)] w-full min-w-0 items-center pr-3 pl-7 text-left outline-none hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
              >
                <span className="truncate">{t.label}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  )
}
