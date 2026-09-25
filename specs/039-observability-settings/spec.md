# Feature Specification: Persist and Hydrate Observability Settings

**Feature Branch**: `039-observability-settings`

**Created**: 2026-09-25

**Status**: Draft

**Input**: User description: "Persist and hydrate observability integration settings. Grafana: ConfigureGrafana is currently a stub that only sends a notification; make it persist the base URL (non-secret) per context and store the API key in the credential store (write-only, never returned to the UI). Prometheus: config (namespace, service, port) is saved per context but nothing reads it back. Add a read path (command plus event) so the Grafana panel and Prometheus form hydrate saved non-secret config on open and show a 'configured' indicator, with the API key shown only as set/unset. Existing Test and Save behavior, validation, and capability gating stay. Fake-mode and live e2e coverage required."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Grafana settings survive and show as configured (Priority: P1)

An operator connects Fathom to their Grafana instance once for a cluster context. Today, Save appears to succeed but nothing is remembered, and the panel is blank on every visit. After this feature, Save keeps the base URL for that context, stores the API key securely, and reopening the panel shows the saved URL and a "configured" indicator.

**Why this priority**: Today Save is a no-op that only notifies, so the integration cannot be used at all. Nothing else in this feature matters until settings persist.

**Independent Test**: Save a Grafana URL and API key for a context, close the panel or restart the app, reopen the panel, and confirm the URL is prefilled, the state reads "configured", and the key is shown only as set.

**Acceptance Scenarios**:

1. **Given** no Grafana settings for a context, **When** the operator opens the panel, **Then** the form is empty and shows "not configured".
2. **Given** a valid URL and API key, **When** the operator saves, **Then** the settings are stored for that context and the panel shows "configured" with the key marked "set".
3. **Given** saved settings, **When** the app is restarted and the panel reopened, **Then** the saved URL is prefilled and the key still shows "set".
4. **Given** saved settings, **When** the operator switches to another context, **Then** that context's own settings (or "not configured") are shown, never the previous context's.
5. **Given** saved settings, **When** the operator saves with the key field left blank, **Then** the URL updates and the existing key is kept.

---

### User Story 2 - Prometheus form remembers its saved endpoint (Priority: P1)

An operator who has saved a Prometheus endpoint (namespace, service, port) for a context sees it prefilled with a "configured" indicator when opening the form, instead of the hard-coded defaults.

**Why this priority**: The configuration is already saved but cannot be seen, so operators cannot tell whether it is set or what it points to.

**Independent Test**: Save a non-default endpoint, reopen the form, and confirm the saved values appear with a "configured" indicator instead of the defaults.

**Acceptance Scenarios**:

1. **Given** a saved endpoint for the context, **When** the form opens, **Then** namespace, service and port are prefilled and marked "configured".
2. **Given** no saved endpoint, **When** the form opens, **Then** the defaults are shown and marked "not configured".
3. **Given** a saved endpoint, **When** the operator edits and saves, **Then** the new values replace the old and are shown on next open.

---

### User Story 3 - Secrets never leave the credential store (Priority: P2)

A security-conscious operator needs confidence that the Grafana API key is never displayed, logged, or sent back to the interface after it is saved.

**Why this priority**: This is a security guarantee that must hold for the P1 stories to be acceptable.

**Independent Test**: Save a key, inspect everything the interface receives, the persisted settings, and the logs, and confirm the key value appears nowhere; only a set/unset flag is exposed.

**Acceptance Scenarios**:

1. **Given** a saved key, **When** the panel loads, **Then** the interface receives only "key set" and never the value.
2. **Given** a saved key, **When** the operator clears it, **Then** the state changes to "key not set" and the stored secret is removed.

---

### Edge Cases

- Credential store unavailable (locked or no keyring): saving the key fails with a clear message, and the URL is not reported as fully configured.
- Saved settings present but the context has since been removed or is unreachable: settings still display, and Test reports the connection failure.
- URL fails validation (not http/https): Save and Test stay disabled, as today.
- Prometheus port outside 1–65535 or empty namespace/service: Save stays disabled, as today.
- Settings changed from a second window or session: the panel reflects the latest saved values on next open.
- Viewer lacks permission to change settings: fields render read-only with the saved values visible, and the existing capability gating applies.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Saving Grafana settings MUST persist the base URL per cluster context, replacing the current notification-only behavior.
- **FR-002**: The Grafana API key MUST be stored only in the credential store and MUST never be returned to the interface, written to persisted settings, or logged.
- **FR-003**: The system MUST provide a way to read saved non-secret Grafana settings for a context, returning the base URL and a key set/unset flag.
- **FR-004**: The system MUST provide a way to read saved Prometheus settings (namespace, service, port) for a context.
- **FR-005**: The Grafana panel and Prometheus form MUST prefill saved values when opened and again when the active context changes.
- **FR-006**: Each form MUST show a "configured" or "not configured" indicator, and Grafana MUST show the key as set or not set.
- **FR-007**: Saving with the API key field blank MUST keep the existing key; the operator MUST be able to explicitly clear it.
- **FR-008**: Existing Test and Save behavior, input validation, and capability gating MUST remain unchanged.
- **FR-009**: Settings MUST be isolated per context.
- **FR-010**: Failures to persist or to read settings MUST show a clear error and MUST NOT leave the UI showing "configured" when nothing was saved.
- **FR-011**: Behavior MUST be covered by automated tests against the fake backend and by live tests against a real cluster.

### Key Entities

- **Grafana settings**: per-context base URL (non-secret) plus an API key held only in the credential store; the interface sees only whether a key is set.
- **Prometheus settings**: per-context namespace, service and port.
- **Configured state**: derived indicator (configured, not configured, key set/unset) shown next to each form.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After saving, 100% of reopenings of the panel, including after an app restart, show the saved values and a "configured" indicator.
- **SC-002**: The Grafana API key value appears in 0 interface payloads, persisted settings files, and log output during a save, reload, and clear cycle.
- **SC-003**: Switching between two contexts with different settings shows the correct settings each time in 100% of trials, with no leakage.
- **SC-004**: An operator can determine whether Grafana and Prometheus are configured for a context within 1 second of opening the panel, with no need to re-enter values.
- **SC-005**: Existing Test, Save and validation scenarios continue to pass with no regressions.

## Assumptions

- Settings are scoped per cluster context, matching how Prometheus is already saved.
- The existing credential store is available on all Tier 1 platforms; when it is not, the failure behavior in Edge Cases applies.
- Only the API key is secret; base URL, namespace, service and port are non-secret and may be shown.
- Multi-user or team-shared settings and centrally managed configuration are out of scope.
- Editing permission continues to follow the existing capability gating; no new roles are introduced.
- Grafana Test behavior is unchanged and uses the saved or entered values as it does today.
