/** Batch C list fixtures (storage + config core), spread into KIND_LIST_EVENTS. */
import type { ResourceListEvent } from '../../src/protocol/types'

export const BATCH_C_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'storage/persistentvolumes': {
    type: 'PersistentVolumesUpdated',
    data: [
      {
        name: 'pv-data',
        capacity: '10Gi',
        access_modes: ['ReadWriteOnce'],
        status: 'Bound',
        storage_class: 'standard',
        reclaim_policy: 'Delete',
        claim_ref: 'default/data',
      },
    ],
  },
  'storage/persistentvolumeclaims': {
    type: 'PersistentVolumeClaimsUpdated',
    data: [
      {
        namespace: 'default',
        name: 'data',
        status: 'Bound',
        volume_name: 'pv-data',
        capacity: '10Gi',
        access_modes: ['ReadWriteOnce'],
        storage_class: 'standard',
      },
    ],
  },
  'storage/storageclasses': {
    type: 'StorageClassesUpdated',
    data: [
      {
        name: 'standard',
        provisioner: 'rancher.io/local-path',
        reclaim_policy: 'Delete',
        volume_binding_mode: 'WaitForFirstConsumer',
        is_default: true,
      },
    ],
  },
  'config/configmaps': {
    type: 'ConfigMapsUpdated',
    data: [{ name: 'app-config', namespace: 'default', keys_count: 2 }],
  },
  'config/secrets': {
    type: 'SecretsUpdated',
    data: [{ name: 'db-creds', namespace: 'default', secret_type: 'Opaque', keys_count: 2 }],
  },
  'config/resourcequotas': {
    type: 'ResourceQuotasUpdated',
    data: [{ name: 'compute', namespace: 'default', hard_limits_count: 2 }],
  },
  'config/limitranges': {
    type: 'LimitRangesUpdated',
    data: [{ name: 'defaults', namespace: 'default', limits_count: 1 }],
  },
  'config/poddisruptionbudgets': {
    type: 'PodDisruptionBudgetsUpdated',
    data: [
      {
        namespace: 'default',
        name: 'web-pdb',
        min_available: '1',
        max_unavailable: null,
        current_healthy: 2,
        desired_healthy: 1,
      },
    ],
  },
}
