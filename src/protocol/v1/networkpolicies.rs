use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NetworkPolicySummary {
    pub name: String,
    pub namespace: String,
    pub pod_selector: String, // Display representation of selector
    pub ingress_rules_count: usize,
    pub egress_rules_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PolicyRule {
    pub protocols: Vec<String>, // "TCP", "UDP", "SCTP"
    pub ports: Vec<u16>,
    pub from_selectors: Vec<RuleSelector>,
    pub to_selectors: Vec<RuleSelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuleSelector {
    pub selector_type: String, // "PodSelector", "NamespaceSelector", "IPBlock"
    pub labels: Option<BTreeMap<String, String>>,
    pub ip_block: Option<String>, // CIDR notation
    pub except: Vec<String>,      // CIDR exceptions
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct NetworkPolicyDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub pod_selector: BTreeMap<String, String>,
    pub policy_types: Vec<String>, // "Ingress", "Egress"
    pub ingress_rules: Vec<PolicyRule>,
    pub egress_rules: Vec<PolicyRule>,
    pub raw_json: String,
}
