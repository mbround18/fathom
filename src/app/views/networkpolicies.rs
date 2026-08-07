use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, NetworkPolicyDetails, NetworkPolicySummary};

/// NetworkPolicy-specific state and rendering for the NetworkPolicies view
pub struct NetworkPoliciesView {
    pub policies: Vec<NetworkPolicySummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<NetworkPolicyDetails>,
    pub table: Table<NetworkPolicySummary>,
}

impl Default for NetworkPoliciesView {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkPoliciesView {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_policies_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<NetworkPolicySummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |p: &NetworkPolicySummary| p.namespace.clone(),
            render: |p: &NetworkPolicySummary| p.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |p: &NetworkPolicySummary| p.name.clone(),
            render: |p: &NetworkPolicySummary| p.name.clone(),
        },
        Column {
            header: "Pod Selector".to_string(),
            width: 200.0,
            sort_value: |p: &NetworkPolicySummary| p.pod_selector.clone(),
            render: |p: &NetworkPolicySummary| p.pod_selector.clone(),
        },
        Column {
            header: "Ingress Rules".to_string(),
            width: 100.0,
            sort_value: |p: &NetworkPolicySummary| p.ingress_rules_count.to_string(),
            render: |p: &NetworkPolicySummary| p.ingress_rules_count.to_string(),
        },
        Column {
            header: "Egress Rules".to_string(),
            width: 100.0,
            sort_value: |p: &NetworkPolicySummary| p.egress_rules_count.to_string(),
            render: |p: &NetworkPolicySummary| p.egress_rules_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |p: &NetworkPolicySummary, search: &str| {
            p.namespace.to_lowercase().contains(search) || p.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|p: &NetworkPolicySummary, search: &str| {
            p.namespace.to_lowercase().contains(search) || p.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, p)) = filtered.get(selected_idx) {
            newly_selected = Some((p.namespace.clone(), p.name.clone()));
        }
    }

    newly_selected
}

fn render_rule(ui: &mut egui::Ui, label: &str, rules: &[crate::protocol::PolicyRule]) {
    if rules.is_empty() {
        return;
    }
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(label);
            ui.separator();
            for rule in rules {
                if !rule.protocols.is_empty() || !rule.ports.is_empty() {
                    ui.monospace(format!(
                        "Protocols: {}  Ports: {}",
                        if rule.protocols.is_empty() {
                            "any".to_string()
                        } else {
                            rule.protocols.join(", ")
                        },
                        if rule.ports.is_empty() {
                            "any".to_string()
                        } else {
                            rule.ports
                                .iter()
                                .map(|p| p.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ));
                }
                for sel in rule.from_selectors.iter().chain(rule.to_selectors.iter()) {
                    let desc = match sel.selector_type.as_str() {
                        "IPBlock" => format!(
                            "IPBlock {}{}",
                            sel.ip_block.clone().unwrap_or_default(),
                            if sel.except.is_empty() {
                                String::new()
                            } else {
                                format!(" except {}", sel.except.join(", "))
                            }
                        ),
                        other => format!(
                            "{} {}",
                            other,
                            sel.labels
                                .as_ref()
                                .map(|l| l
                                    .iter()
                                    .map(|(k, v)| format!("{}={}", k, v))
                                    .collect::<Vec<_>>()
                                    .join(","))
                                .unwrap_or_default()
                        ),
                    };
                    ui.monospace(format!("  {}", desc));
                }
            }
        });
    });
    ui.add_space(8.0);
}

impl ViewComponent for NetworkPoliciesView {
    fn id(&self) -> &'static str {
        "network_policies"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.policies.clone());

        let newly_selected = render_policies_grid(ui, "Network Policies", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectNetworkPolicy {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::NetworkPoliciesUpdated(policies),
            ) => {
                self.policies = policies.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::NetworkPolicyDetailsUpdated(details),
            ) => {
                self.details = Some(details.clone());
            }
            _ => {}
        }
    }

    fn has_details(&self) -> bool {
        self.selected.is_some()
    }
    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Network Policy");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteNetworkPolicy(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected = None;
                    self.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.details {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Overview");
                        ui.separator();
                        ui.monospace(format!("Policy Types: {}", details.policy_types.join(", ")));
                    });
                });
                ui.add_space(8.0);

                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🏷️ Pod Selector");
                            ui.separator();
                            for (k, v) in &details.pod_selector {
                                ui.monospace(format!("{}={}", k, v));
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                render_rule(ui, "⬇️ Ingress Rules", &details.ingress_rules);
                render_rule(ui, "⬆️ Egress Rules", &details.egress_rules);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading network policy details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshNetworkPolicies)]
    }
}
