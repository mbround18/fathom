/**
 * Section navigator (T065): collapsible groups from `NAV_GROUPS`, keyed by backend section keys.
 * Used by the nav slot (laptop+) and the drawer (phone/tablet). Expanded groups persist through
 * `SaveUiConfig` (`expanded_groups`).
 */
import { CaretRightIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'

import { useShallow } from 'zustand/react/shallow'

import type { SectionKey } from '@/protocol/types'
import { cn } from '@/lib/utils'
import { descriptorFor } from '@/resources'
import { integrationInstalled, NOT_INSTALLED_HINT } from '@/resources/integrationStatus'
import { DEFAULT_EXPANDED_GROUPS, NAV_GROUPS, NAV_TOOLS, PANEL_SECTIONS } from '@/resources/nav'
import { useStore } from '@/store'

import { ALL_TOOL_TABS } from './shell/stack-tabs'
import { useOpenTool } from './shell/useOpenTool'

export function NavTree() {
  const navigate = useStore((s) => s.navigate)
  const openTool = useOpenTool()
  const setExpandedGroups = useStore((s) => s.setExpandedGroups)
  const expanded = useStore((s) =>
    s.prefs.loaded ? s.prefs.expandedGroups : (DEFAULT_EXPANDED_GROUPS as string[]),
  )
  const active = useStore((s) => {
    const id = s.layout.focusedList ?? s.layout.slotPanes.find((p) => s.panes[p]?.kind === 'list')
    return id ? s.panes[id]?.route?.section : undefined
  })

  const installed = useStore(
    useShallow((s) =>
      Object.fromEntries(
        NAV_GROUPS.flatMap((g) => g.sections).map((x) => [x.key, integrationInstalled(s, x.key)]),
      ),
    ),
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

  const toggle = (label: string) =>
    setExpandedGroups(
      expanded.includes(label) ? expanded.filter((g) => g !== label) : [...expanded, label],
    )

  return (
    <nav aria-label="Sections" className="flex flex-col py-1">
      {NAV_GROUPS.map((group) => {
        const open = expanded.includes(group.label)
        const id = `nav-group-${group.label.replace(/\s+/g, '-').toLowerCase()}`
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
                {group.sections.map((s) => {
                  const current = active === s.key
                  const ported = descriptorFor(s.key) !== undefined || PANEL_SECTIONS.has(s.key)
                  const absent = installed[s.key] === false
                  return (
                    <li key={s.key} className="flex">
                      <button
                        type="button"
                        aria-current={current ? 'page' : undefined}
                        data-section={s.key}
                        data-installed={absent ? 'false' : undefined}
                        title={absent ? NOT_INSTALLED_HINT : undefined}
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
                    </li>
                  )
                })}
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
