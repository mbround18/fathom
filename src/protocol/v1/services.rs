use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSummary {
    pub name: String,
    pub namespace: String,
    pub service_type: String, // "ClusterIP", "NodePort", "LoadBalancer", "ExternalName"
    pub cluster_ip: String,
    pub external_ips: Vec<String>,
    pub port_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target_port: Option<String>,
    pub protocol: String,
    pub node_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub service_type: String,
    pub cluster_ip: String,
    pub cluster_ips: Vec<String>,
    pub external_ips: Vec<String>,
    pub external_name: Option<String>,
    pub selector: BTreeMap<String, String>,
    pub ports: Vec<ServicePort>,
    pub session_affinity: String,
    pub load_balancer_ip: Option<String>,
    pub load_balancer_ingress_hosts: Vec<String>,
    pub raw_json: String,
}
