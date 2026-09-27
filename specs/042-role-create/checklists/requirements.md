# Requirements checklist: 042-role-create

- [x] FR-001 New Role / New ClusterRole in the panes, gated on `create`: mock e2e "creates a Role…"
- [x] FR-002 Rule rows with groups, resources, verb checkboxes, extras, resourceNames, nonResourceURLs; add/remove: unit + mock e2e
- [x] FR-003 Discovery-fed pickers with free text: live spec checks `configmaps` offered and `nodes` not offered for a Role, offered for a ClusterRole
- [x] FR-004 Presets: unit `presets`
- [x] FR-005 Validation: unit `validation`
- [x] FR-006 Exact manifest with Copy/Download; diff on edit: mock e2e
- [x] FR-007 Confirm-gated, denied → disabled with reason: mock e2e "denied ClusterRole"
- [x] FR-008 `CreateResourceYaml` / `ApplyResourceYaml` with `expected_resource_version`: mock e2e (edit asserts version `42`)
- [x] FR-009 Rules-only patch, `system:` and aggregation warnings: unit `edit mode` + mock e2e
- [x] FR-010 Escalation explained with missing permissions: unit + mock e2e
- [x] FR-011 List refresh after write, inspector reload after edit: live spec (row appears after create; edit via inspector)
- [x] FR-012 Bind shortcut pre-fills the binding form: mock e2e
- [x] FR-013 Read-only, group-tolerant discovery: `mutation.rs` classification + serde tests; live picker check
