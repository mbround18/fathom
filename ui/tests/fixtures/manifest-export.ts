/**
 * 040-manifest-export fake replies: `FetchManifests` reads the generated Deployments and Pods of
 * the current context (anything else is "not found"), and the kubeconfig export commands answer
 * for `kind-a` (embedded bearer token) and `kind-b` (exec plugin). No real secret appears here:
 * the backend redacts before anything reaches the UI, so the fake only ever sends `<redacted>`.
 */
import type {
  BackendCmd,
  EventBatch,
  FetchedManifest,
  KubeconfigExportPreview,
  ManifestFetchError,
  PodSummary,
} from '../../src/protocol/types'
import { batch, system } from './builders'
import { deploymentDetails, genDeployments, podDetails, type FixtureContext } from './gen-pods'

type Cmd<T extends BackendCmd['type']> = Extract<BackendCmd, { type: T }>

export function fetchManifestsReply(
  cmd: Cmd<'FetchManifests'>,
  context: FixtureContext | null,
  podRows: readonly PodSummary[],
): EventBatch[] {
  const manifests: FetchedManifest[] = []
  const errors: ManifestFetchError[] = []
  cmd.data.objects.forEach((ref, index) => {
    let raw: string | undefined
    if (context && ref.kind === 'Deployment') {
      const d = genDeployments(context).find(
        (x) => x.namespace === ref.namespace && x.name === ref.name,
      )
      if (d) raw = deploymentDetails(d, podRows).raw_json
    } else if (context && ref.kind === 'Pod') {
      const p = podRows.find((x) => x.namespace === ref.namespace && x.name === ref.name)
      if (p) raw = podDetails(p).raw_json
    }
    if (raw === undefined) {
      errors.push({
        index,
        message: `${ref.kind.toLowerCase()}s "${ref.name}" not found`,
      })
    } else {
      manifests.push({ index, raw_json: raw })
    }
  })
  return [
    batch([
      system(90, {
        type: 'ManifestsFetched',
        data: { request_id: cmd.data.request_id, manifests, errors },
      }),
    ]),
  ]
}

const HOME = '/home/fathom'

function preview(context: string): KubeconfigExportPreview | undefined {
  const cluster = `  cluster:\n    certificate-authority-data: DATA+OMITTED\n    server: https://127.0.0.1:6443\n  name: ${context}\n`
  const head = `apiVersion: v1\nkind: Config\ncurrent-context: ${context}\nclusters:\n- ${cluster.trimStart()}contexts:\n- context:\n    cluster: ${context}\n    user: ${context}-user\n  name: ${context}\n`
  if (context === 'kind-a') {
    return {
      context,
      redacted_yaml: `${head}users:\n- name: ${context}-user\n  user:\n    token: <redacted>\n`,
      credential: { type: 'Token' },
      embedded_secret_fields: ['token'],
      file_references: [],
      warnings: [],
      suggested_path: `${HOME}/${context}.kubeconfig`,
      source_path: `${HOME}/.kube/config`,
    }
  }
  if (context === 'kind-b') {
    return {
      context,
      redacted_yaml: `${head}users:\n- name: ${context}-user\n  user:\n    exec:\n      apiVersion: client.authentication.k8s.io/v1\n      command: kubelogin\n      args:\n      - get-token\n`,
      credential: { type: 'Exec', data: { command: 'kubelogin' } },
      embedded_secret_fields: [],
      file_references: [],
      warnings: [
        'Uses the exec plugin "kubelogin": it must be installed wherever this file is used.',
      ],
      suggested_path: `${HOME}/${context}.kubeconfig`,
      source_path: `${HOME}/.kube/config`,
    }
  }
  return undefined
}

export function previewKubeconfigReply(cmd: Cmd<'PreviewKubeconfigExport'>): EventBatch[] {
  const data = preview(cmd.data.context)
  return [
    batch([
      data
        ? system(91, { type: 'KubeconfigExportPreviewed', data })
        : system(91, {
            type: 'KubeconfigExportFinished',
            data: {
              context: cmd.data.context,
              ok: false,
              path: null,
              embedded_credentials_written: false,
              message: `context "${cmd.data.context}" not found in kubeconfig`,
            },
          }),
    ]),
  ]
}

export function exportKubeconfigReply(cmd: Cmd<'ExportKubeconfig'>): EventBatch[] {
  const p = preview(cmd.data.context)
  const embedded =
    cmd.data.include_embedded_credentials && (p?.embedded_secret_fields.length ?? 0) > 0
  return [
    batch([
      system(92, {
        type: 'KubeconfigExportFinished',
        data: {
          context: cmd.data.context,
          ok: p !== undefined,
          path: p ? cmd.data.path : null,
          embedded_credentials_written: p !== undefined && embedded,
          message: p ? null : `context "${cmd.data.context}" not found in kubeconfig`,
        },
      }),
    ]),
  ]
}
