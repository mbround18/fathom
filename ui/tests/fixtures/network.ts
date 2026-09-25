/** Batch B (network) fixtures: list events and details. */
import type {
  EndpointDetails,
  EndpointSummary,
  IngressClassSummary,
  IngressDetails,
  IngressSummary,
  NetworkPolicyDetails,
  NetworkPolicySummary,
  ResourceListEvent,
} from '../../src/protocol/types'

export const ENDPOINTS: EndpointSummary[] = [
  {
    name: 'web',
    namespace: 'default',
    ready_addresses_count: 2,
    not_ready_addresses_count: 1,
    port_count: 1,
  },
]
export const INGRESSES: IngressSummary[] = [
  {
    name: 'web',
    namespace: 'default',
    ingress_class: 'nginx',
    hosts_count: 1,
    load_balancer_ips: ['10.0.0.5'],
  },
  {
    name: 'pending',
    namespace: 'default',
    ingress_class: null,
    hosts_count: 0,
    load_balancer_ips: [],
  },
]
export const INGRESS_CLASSES: IngressClassSummary[] = [
  { name: 'nginx', controller: 'k8s.io/ingress-nginx' },
]
export const NETPOLS: NetworkPolicySummary[] = [
  {
    name: 'deny-all',
    namespace: 'default',
    pod_selector: 'app=web',
    ingress_rules_count: 1,
    egress_rules_count: 1,
  },
]

export const NETWORK_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'network/endpoints': { type: 'EndpointsUpdated', data: ENDPOINTS },
  'network/ingress': { type: 'IngressesUpdated', data: INGRESSES },
  'network/ingressclasses': { type: 'IngressClassesUpdated', data: INGRESS_CLASSES },
  'network/networkpolicies': { type: 'NetworkPoliciesUpdated', data: NETPOLS },
}

export const INGRESS_DETAILS: IngressDetails = {
  namespace: 'default',
  name: 'web',
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: {},
  annotations: {},
  ingress_class: 'nginx',
  rules: [
    {
      host: 'app.example.com',
      paths: [
        {
          path: '/api',
          path_type: 'Prefix',
          backend: {
            service_name: 'api',
            service_port: 8080,
            resource_kind: null,
            resource_name: null,
          },
        },
      ],
    },
  ],
  default_backend: null,
  load_balancer_ingress: [],
  tls_hosts: ['app.example.com'],
  raw_json: '{"kind":"Ingress"}',
}

export const NETPOL_DETAILS: NetworkPolicyDetails = {
  namespace: 'default',
  name: 'deny-all',
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: {},
  annotations: {},
  pod_selector: { app: 'web' },
  policy_types: ['Ingress', 'Egress'],
  ingress_rules: [
    {
      protocols: ['TCP'],
      ports: [80],
      from_selectors: [
        { selector_type: 'PodSelector', labels: { role: 'fe' }, ip_block: null, except: [] },
      ],
      to_selectors: [],
    },
  ],
  egress_rules: [
    {
      protocols: [],
      ports: [],
      from_selectors: [],
      to_selectors: [
        {
          selector_type: 'IPBlock',
          labels: null,
          ip_block: '10.0.0.0/8',
          except: ['10.1.0.0/16'],
        },
      ],
    },
  ],
  raw_json: '{"kind":"NetworkPolicy"}',
}

export const ENDPOINT_DETAILS: EndpointDetails = {
  namespace: 'default',
  name: 'web',
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: {},
  annotations: {},
  subsets: [
    {
      ready_addresses: [
        { ip: '10.1.0.1', hostname: null, node_name: 'n1', target_ref: 'Pod/default/web-1' },
      ],
      not_ready_addresses: [{ ip: '10.1.0.2', hostname: null, node_name: null, target_ref: null }],
      ports: [{ name: 'http', port: 80, protocol: 'TCP' }],
    },
  ],
  raw_json: '{"kind":"Endpoints"}',
}
