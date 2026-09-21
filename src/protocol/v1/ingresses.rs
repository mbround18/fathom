use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressSummary {
    pub name: String,
    pub namespace: String,
    pub ingress_class: Option<String>,
    pub hosts_count: usize,
    pub load_balancer_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressBackend {
    pub service_name: Option<String>,
    pub service_port: Option<u16>,
    pub resource_kind: Option<String>, // For resource-type backends
    pub resource_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressRule {
    pub host: Option<String>,
    pub paths: Vec<IngressPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressPath {
    pub path: Option<String>,
    pub path_type: String, // "Exact", "Prefix", "ImplementationSpecific"
    pub backend: IngressBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressLoadBalancerIngress {
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub ports: Vec<IngressLoadBalancerPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressLoadBalancerPort {
    pub port: u16,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IngressDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub ingress_class: Option<String>,
    pub rules: Vec<IngressRule>,
    pub default_backend: Option<IngressBackend>,
    pub load_balancer_ingress: Vec<IngressLoadBalancerIngress>,
    pub tls_hosts: Vec<String>,
    pub raw_json: String,
}
