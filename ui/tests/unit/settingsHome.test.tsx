/**
 * 043 US3/US4: the Settings pages list (header menu and phone overflow), and the daemon switch
 * and Cluster history copy that answer "what is the on/off switch for" (feedback #12).
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { settingsItems } from '@/components/settings/settings-items'
import { DaemonSwitch } from '@/features/daemon/DaemonSwitch'
import { DaemonTimelinePanel } from '@/features/daemon/DaemonTimelinePanel'
import {
  CLUSTER_HISTORY_SUBTITLE,
  DAEMON_SWITCH_DESCRIPTION,
  DAEMON_SWITCH_LABEL,
} from '@/features/daemon/state'
import { navEntry } from '@/resources/nav'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  useStore.setState((s) => ({ session: { ...s.session, activeContext: 'kind-a' } }))
})

describe('settingsItems', () => {
  it('lists the Settings pages in menu order on the desktop', () => {
    expect(settingsItems(false).map((i) => i.label)).toEqual([
      'Cluster settings',
      'Sign-in',
      'Team',
      'Connection profiles',
      'App settings',
      'Local clusters',
      'Audit log',
    ])
  })

  it('leaves Local clusters out in hosted mode', () => {
    expect(settingsItems(true).map((i) => i.section)).not.toContain('settings/local-clusters')
    expect(settingsItems(true)).toHaveLength(6)
  })

  it('matches the navigator Settings group labels', () => {
    for (const item of settingsItems(false)) {
      expect(navEntry(item.section)?.label).toBe(item.label)
    }
  })
})

describe('DaemonSwitch', () => {
  it('has a name and description that say what on and off do', () => {
    render(<DaemonSwitch />)
    const toggle = screen.getByRole('switch', { name: DAEMON_SWITCH_LABEL })
    expect(toggle).toHaveAccessibleDescription(DAEMON_SWITCH_DESCRIPTION)
    expect(toggle).not.toBeChecked()
  })

  it('sends the auto preference for the active context when turned on', () => {
    const setDaemonPreference = vi.fn()
    useStore.setState({ setDaemonPreference })
    render(<DaemonSwitch />)
    fireEvent.click(screen.getByRole('switch', { name: DAEMON_SWITCH_LABEL }))
    expect(setDaemonPreference).toHaveBeenCalledWith('kind-a', 'auto')
  })

  it('renders nothing without a context', () => {
    useStore.setState((s) => ({ session: { ...s.session, activeContext: undefined } }))
    const { container } = render(<DaemonSwitch />)
    expect(container).toBeEmptyDOMElement()
  })
})

describe('DaemonTimelinePanel (Cluster history)', () => {
  it('opens with the subtitle and explains how to turn the daemon on when it is off', () => {
    render(<DaemonTimelinePanel />)
    expect(screen.getByTestId('cluster-history-subtitle')).toHaveTextContent(
      CLUSTER_HISTORY_SUBTITLE,
    )
    const why = screen.getByTestId('cluster-history-unavailable')
    expect(why).toHaveTextContent(DAEMON_SWITCH_LABEL)
    expect(why).toHaveTextContent('Settings › Cluster settings')
    expect(why).toHaveTextContent('kubectl apply -k k8s/overlays/daemon')
  })

  it('"Open Cluster settings" navigates to the Cluster settings page', () => {
    const navigate = vi.fn()
    useStore.setState({ navigate })
    render(<DaemonTimelinePanel />)
    fireEvent.click(screen.getByTestId('cluster-history-open-settings'))
    expect(navigate).toHaveBeenCalledWith('integrations/cluster-settings')
  })
})
