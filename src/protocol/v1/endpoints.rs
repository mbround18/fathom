use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EndpointSummary {
    pub name: String,
    pub namespace: String,
    pub ready_addresses_count: usize,
    pub not_ready_addresses_count: usize,
    pub port_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EndpointAddress {
    pub ip: String,
    pub hostname: Option<String>,
    pub node_name: Option<String>,
    pub target_ref: Option<String>, // "Pod/namespace/name" or similar
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EndpointPort {
    pub name: Option<String>,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EndpointSubset {
    pub ready_addresses: Vec<EndpointAddress>,
    pub not_ready_addresses: Vec<EndpointAddress>,
    pub ports: Vec<EndpointPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EndpointDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub subsets: Vec<EndpointSubset>,
    pub raw_json: String,
}
