/** Deployments (T067), built from the shared workload descriptor. */
import { workloadDescriptor } from './workloads'

export const deployments = workloadDescriptor({
  section: 'workloads/deployments',
  kind: 'Deployment',
  label: 'Deployments',
  listEvent: 'DeploymentsUpdated',
  readEvent: 'DeploymentDetailsUpdated',
  refresh: { type: 'RefreshDeployments' },
  select: (namespace, name) => ({ type: 'SelectDeployment', data: { namespace, name } }),
  scale: (namespace, name, replicas) => ({
    type: 'ScaleDeployment',
    data: { namespace, name, replicas },
  }),
  restart: (namespace, name) => ({ type: 'RestartDeployment', data: { namespace, name } }),
  delete: (namespace, name) => ({ type: 'DeleteDeployment', data: { namespace, name } }),
})
