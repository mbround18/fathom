/**
 * 035-cloud-auth-plugins: store handlers for `ContextStatusUpdated.plugin_error` and
 * `CloudAuthDetected`, and the switcher / settings rendering built on them.
 */
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { CloudAuthSection } from '@/components/panels/CloudAuthSection'
import { FederationContextSwitcher } from '@/components/shell/FederationContextSwitcher'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'

import { federationContexts } from '../fixtures'
import { batch, system } from '../fixtures/builders'

const pluginError = {
  kind: 'pluginMissing' as const,
  context: 'kind-a',
  provider: 'gke' as const,
  command: 'gke-gcloud-auth-plugin',
  message: '`gke-gcloud-auth-plugin` was not found',
  hint: 'gcloud components install gke-gcloud-auth-plugin',
}

const info = {
  context: 'kind-a',
  provider: 'gke' as const,
  command: 'gke-gcloud-auth-plugin',
  installed: false,
  resolved_path: null,
  exec_policy_allowed: true,
  exec_policy: 'allowKnownOnly' as const,
  install_hint: pluginError.hint,
}

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

describe('cloud auth store + UI', () => {
  it('keeps the typed plugin error on the context status entry', () => {
    ingest(federationContexts)
    ingest(
      batch([
        system(5, {
          type: 'ContextStatusUpdated',
          data: {
            context: 'kind-a',
            status: 'Error',
            error: pluginError.message,
            plugin_error: pluginError,
          },
        }),
      ]),
    )
    expect(useStore.getState().session.contextStatuses['kind-a']?.pluginError).toEqual(pluginError)
  })

  it('indexes CloudAuthDetected by context', () => {
    ingest(batch([system(6, { type: 'CloudAuthDetected', data: { infos: [info] } })]))
    expect(useStore.getState().session.cloudAuth['kind-a']).toEqual(info)
  })

  it('switcher tooltip carries the message, hint and provider badge', () => {
    ingest(federationContexts)
    ingest(
      batch([
        system(5, {
          type: 'ContextStatusUpdated',
          data: {
            context: 'kind-a',
            status: 'Error',
            error: pluginError.message,
            plugin_error: pluginError,
          },
        }),
        system(6, { type: 'CloudAuthDetected', data: { infos: [info] } }),
      ]),
    )
    render(<FederationContextSwitcher />)
    const item = screen
      .getAllByTestId('federation-context-item')
      .find((el) => el.getAttribute('data-context-name') === 'kind-a')
    expect(item?.getAttribute('title')).toContain('gcloud components install')
    expect(item?.getAttribute('title')).toContain('[gke, plugin missing]')
  })

  it('settings section shows provider, missing plugin and install hint', () => {
    ingest(federationContexts)
    useStore.setState((s) => ({
      session: { ...s.session, activeContext: 'kind-a', cloudAuth: { 'kind-a': info } },
    }))
    render(<CloudAuthSection />)
    expect(screen.getByTestId('cloud-auth-provider').textContent).toBe('Google GKE')
    expect(screen.getByTestId('cloud-auth-installed').textContent).toContain('not found')
    expect(screen.getByTestId('cloud-auth-hint').textContent).toContain('gcloud components')
    expect(screen.getByTestId('cloud-auth-policy-toggle')).toBeTruthy()
  })
})
