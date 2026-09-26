/**
 * 040-manifest-export (T020, T034): the export dialog reads fresh by request id and renders a
 * clean/raw bundle; the kubeconfig dialog shows only the redacted preview, flags embedded
 * credentials and writes only after the confirm step.
 */
import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { useExportDialog, useKubeconfigExport } from '@/features/manifest-export/dialog-stores'
import { KubeconfigExportDialogHost } from '@/features/manifest-export/KubeconfigExportDialog'
import { MANIFEST_EXPORT_MAX_OBJECTS } from '@/features/manifest-export/limits'
import { ManifestExportDialogHost } from '@/features/manifest-export/ManifestExportDialog'
import { initialManifestExportState, reduceManifestExport } from '@/features/manifest-export/state'
import type { BackendCmd, ManifestObjectRef, SystemEvent } from '@/protocol/types'
import { useStore } from '@/store'

let sent: BackendCmd[] = []

function deliver(event: SystemEvent) {
  act(() => {
    const next = structuredClone(useStore.getState().manifestExport)
    reduceManifestExport(next, event)
    useStore.setState({ manifestExport: next })
  })
}

const deployment = (name: string) =>
  JSON.stringify({
    apiVersion: 'apps/v1',
    kind: 'Deployment',
    metadata: {
      name,
      namespace: 'default',
      uid: 'u-1',
      resourceVersion: '42',
      creationTimestamp: '2026-09-18T11:02:00Z',
      annotations: { 'deployment.kubernetes.io/revision': '3' },
    },
    spec: { replicas: 1 },
    status: { replicas: 1 },
  })

const ref = (name: string): ManifestObjectRef => ({
  api_version: 'apps/v1',
  kind: 'Deployment',
  namespace: 'default',
  name,
})

beforeEach(() => {
  sent = []
  useStore.setState({
    send: (c: BackendCmd) => void sent.push(c),
    manifestExport: initialManifestExportState(),
  })
})

afterEach(() => {
  useExportDialog.setState({ request: null })
  useKubeconfigExport.setState({ open: false, context: null })
})

describe('ManifestExportDialog', () => {
  it('fetches by request id and renders a clean bundle, raw on toggle', async () => {
    const user = userEvent.setup()
    act(() => {
      useExportDialog.setState({
        request: {
          id: 'export-t-1',
          kind: 'Deployment',
          refs: [ref('api'), ref('gone'), ref('web')],
        },
      })
    })
    render(<ManifestExportDialogHost />)
    expect(sent).toEqual([
      {
        type: 'FetchManifests',
        data: { request_id: 'export-t-1', objects: [ref('api'), ref('gone'), ref('web')] },
      },
    ])
    // A reply for another dialog is ignored.
    deliver({
      type: 'ManifestsFetched',
      data: {
        request_id: 'other',
        manifests: [{ index: 0, raw_json: deployment('x') }],
        errors: [],
      },
    })
    expect(screen.getByTestId('manifest-export-count')).toHaveTextContent('Reading 3 objects')

    deliver({
      type: 'ManifestsFetched',
      data: {
        request_id: 'export-t-1',
        manifests: [
          { index: 0, raw_json: deployment('api') },
          { index: 2, raw_json: deployment('web') },
        ],
        errors: [{ index: 1, message: 'deployments.apps "gone" not found' }],
      },
    })
    expect(screen.getByTestId('manifest-export-count')).toHaveTextContent('2 of 3 objects')
    expect(screen.getByTestId('manifest-export-error')).toHaveTextContent(
      'default/gone: deployments.apps "gone" not found',
    )
    const yaml = screen.getByTestId('manifest-export-yaml').textContent
    expect(yaml.split('\n---\n')).toHaveLength(2)
    expect(yaml).not.toMatch(/resourceVersion|uid:|status:|creationTimestamp|revision/)

    await user.click(screen.getByTestId('manifest-clean'))
    const raw = screen.getByTestId('manifest-export-yaml').textContent
    expect(raw).toContain('resourceVersion')
    expect(raw).toContain('status:')
  })

  it('refuses more than the cap without reading anything', () => {
    const refs = Array.from({ length: MANIFEST_EXPORT_MAX_OBJECTS + 1 }, (_, i) =>
      ref(`d${String(i)}`),
    )
    act(() => {
      useExportDialog.setState({ request: { id: 'export-t-2', kind: 'Deployment', refs } })
    })
    render(<ManifestExportDialogHost />)
    expect(screen.getByTestId('manifest-export-error')).toHaveTextContent('export limit is 500')
    expect(sent).toEqual([])
  })
})

describe('KubeconfigExportDialog', () => {
  beforeEach(() => {
    useStore.setState((s) => ({
      session: { ...s.session, contexts: ['kind-a', 'kind-b'], activeContext: 'kind-a' },
    }))
  })

  it('previews redacted, flags embedded credentials and writes only after confirm', async () => {
    const user = userEvent.setup()
    act(() => {
      useKubeconfigExport.getState().show(null)
    })
    render(<KubeconfigExportDialogHost />)
    expect(sent).toEqual([{ type: 'PreviewKubeconfigExport', data: { context: 'kind-a' } }])
    expect(screen.getByTestId('kubeconfig-export')).toBeDisabled()

    deliver({
      type: 'KubeconfigExportPreviewed',
      data: {
        context: 'kind-a',
        redacted_yaml: 'users:\n- name: u\n  user:\n    token: <redacted>\n',
        credential: { type: 'Token' },
        embedded_secret_fields: ['token'],
        file_references: [],
        warnings: [],
        suggested_path: '/home/me/kind-a.kubeconfig',
        source_path: '/home/me/.kube/config',
      },
    })
    expect(screen.getByTestId('kubeconfig-credential')).toHaveTextContent('Bearer token')
    expect(screen.getByTestId('kubeconfig-embedded')).toHaveTextContent('token')
    expect(screen.getByTestId('kubeconfig-preview')).toHaveTextContent('<redacted>')
    expect(screen.getByTestId('kubeconfig-path')).toHaveValue('/home/me/kind-a.kubeconfig')
    // FR-026: no copy of a kubeconfig.
    expect(screen.queryByTestId('manifest-copy')).toBeNull()

    await user.click(screen.getByTestId('kubeconfig-include-credentials'))
    await user.click(screen.getByTestId('kubeconfig-export'))
    const confirm = screen.getByTestId('confirm-dialog')
    expect(within(confirm).getByTestId('kubeconfig-confirm-detail')).toHaveTextContent(
      'Includes embedded credentials (token)',
    )
    expect(sent.map((c) => c.type)).toEqual(['PreviewKubeconfigExport'])

    await user.click(within(confirm).getByTestId('confirm-submit'))
    expect(sent.at(-1)).toEqual({
      type: 'ExportKubeconfig',
      data: {
        context: 'kind-a',
        path: '/home/me/kind-a.kubeconfig',
        include_embedded_credentials: true,
        overwrite: false,
      },
    })
    deliver({
      type: 'KubeconfigExportFinished',
      data: {
        context: 'kind-a',
        ok: true,
        path: '/home/me/kind-a.kubeconfig',
        embedded_credentials_written: true,
        message: null,
      },
    })
    expect(screen.getByTestId('kubeconfig-result')).toHaveAttribute('data-ok', 'true')
    expect(screen.getByTestId('kubeconfig-result')).toHaveTextContent('with embedded credentials')
  })

  it('shows a preview failure (e.g. hosted refusal) as an error', () => {
    act(() => {
      useKubeconfigExport.getState().show('kind-b')
    })
    render(<KubeconfigExportDialogHost />)
    expect(sent).toEqual([{ type: 'PreviewKubeconfigExport', data: { context: 'kind-b' } }])
    deliver({
      type: 'KubeconfigExportFinished',
      data: {
        context: 'kind-b',
        ok: false,
        path: null,
        embedded_credentials_written: false,
        message: 'Kubeconfig export is not available on the hosted server.',
      },
    })
    expect(screen.getByTestId('kubeconfig-error')).toHaveTextContent('not available')
    expect(screen.getByTestId('kubeconfig-export')).toBeDisabled()
  })
})
