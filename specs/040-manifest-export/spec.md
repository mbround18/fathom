# Feature Specification: Manifest Export (never blackbox the cluster)

**Feature Branch**: `040-manifest-export`

**Created**: 2026-09-26

**Status**: Draft

**Input**: User description: "040-manifest-export: Fathom must never blackbox the cluster. (1) Per-resource Copy YAML / Download YAML in Inspector and list row actions with a 'clean for re-apply' option stripping server-managed fields; (2) bundle export of a multi-selection as one multi-document YAML; (3) show the generated manifest (copy/download) in every create/edit flow before apply (RBAC role binding form, kind/helm panels where applicable); (4) kubeconfig export of current or selected context as a standalone file with explicit confirm, embedded-credential flagging, exec/auth-provider preserved as-is, never logging tokens. Export is read-only and works for read-only identities. Out of scope: Helm/kustomize generation, OIDC→kubeconfig sync."

Backlog source: `docs/FEATURE_BACKLOG.md`, "Priority principle: never blackbox the cluster — manifest export".

## Clarifications

### Session 2026-09-26 (self-resolved with defaults; recorded for review)

- Q: Bundle format — one multi-document YAML, a zip of files, or both? → A: One multi-document YAML file only. It is what `kubectl apply -f` consumes directly and needs no new archive dependency. A zip of per-object files stays in the backlog.
- Q: What does "clean for re-apply" remove exactly? → A: The fixed rule set in FR-006..FR-009. It is on by default for Download and Copy from export menus; the Inspector YAML tab's Copy/Download follow a visible toggle that defaults to clean.
- Q: What happens to `ownerReferences` in clean mode? → A: Removed, because they bind to the owner's cluster-specific UID. Objects that had a controller owner are listed with a warning ("owned by ReplicaSet web-7d9f; re-applying it directly creates an unmanaged copy — export the owner instead").
- Q: Secrets? → A: Secret values are never exported. Every value under `data` and `stringData` is replaced by a placeholder, matching what the Inspector already shows, and the dialog says so. The `kubectl.kubernetes.io/last-applied-configuration` annotation is always removed from Secrets, raw or clean, because it can carry the plaintext secret. A redacted Secret fails server validation if re-applied, so it cannot silently overwrite real data.
- Q: Which create/edit flows show the manifest in 040? → A: Flows whose manifest already exists on the client: the YAML edit tab (edit/apply), RoleBinding/ClusterRoleBinding create and edit, OIDC role assignment, Helm upgrade and rollback previews (rendered target manifest), and kind cluster creation (the kind config). Node shell (037) and debug containers (025) build their objects inside the backend. Exposing those needs protocol work, so they stay in the backlog.
- Q: How does the kubeconfig file reach disk, and may the token appear on screen? → A: The backend writes the file directly, with owner-only permissions (0600), to a path the operator chooses (defaulting to `~/.kube/fathom-<context>.yaml`). The interface only ever receives a redacted preview, so credential values never cross the protocol and never render. There is no clipboard copy for kubeconfigs.
- Q: Scoped ServiceAccount kubeconfig (TokenRequest minting) and merge into `~/.kube/config`? → A: Out of scope for 040; they stay in the backlog. 040 exports an existing context as-is.
- Q: Hosted (browser) mode? → A: Manifest export works in hosted mode as the signed-in user. Kubeconfig export is desktop-only: the constitution forbids desktop-only kubeconfig features in hosted mode, and a hosted server's kubeconfig belongs to the server, not the user. The entry point is hidden there and the backend refuses the command.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Copy or download one object's YAML, clean for re-apply (Priority: P1)

An operator looking at a Deployment in Fathom wants the manifest out of the app, either to commit to Git or to re-apply elsewhere. From the Inspector's YAML tab, or from the object's row menu in any list, they copy it to the clipboard or download it as a `.yaml` file. By default the manifest is cleaned of server-managed fields, so `kubectl apply -f` accepts it as-is.

**Why this priority**: This is the core "never blackbox" guarantee. Without it the only way to get YAML out is to go through the edit/apply flow.

**Independent Test**: Open a Deployment, copy its YAML with "clean for re-apply" on, and confirm the clipboard holds a manifest without `status`, `managedFields`, `resourceVersion`, `uid`, `creationTimestamp` or the last-applied annotation. Toggle clean off and confirm the raw object is copied. Download and confirm a file named `<kind>-<namespace>-<name>.yaml` with the same content.

**Acceptance Scenarios**:

1. **Given** an object open in the Inspector, **When** the operator opens the YAML tab, **Then** Copy, Download and a "Clean for re-apply" toggle (on by default) are shown above the YAML.
2. **Given** clean is on, **When** the operator presses Copy, **Then** the clipboard holds the cleaned manifest and a confirmation appears.
3. **Given** clean is off, **When** the operator presses Download, **Then** a file with the object exactly as the cluster returned it is saved.
4. **Given** a row in any list of an exportable kind, **When** the operator opens its row menu and picks "Export YAML…", **Then** an export dialog shows the fetched manifest with the same clean toggle, Copy and Download.
5. **Given** an identity that may only read, **When** it exports, **Then** export works (it needs no write permission) and is never shown as disabled for lack of write access.
6. **Given** a Secret, **When** it is exported, **Then** every data value is a placeholder and the dialog says values were redacted.
7. **Given** an object with a controller owner, **When** it is exported clean, **Then** the dialog warns that re-applying it directly creates an unmanaged copy and names the owner.

---

### User Story 2 - Export a multi-selection as one multi-document YAML (Priority: P1)

An operator selects several ConfigMaps, or every Deployment in a namespace, and exports them as one file of `---`-separated documents that `kubectl apply -f` applies in one go.

**Why this priority**: Getting a set of objects out (for example the RBAC objects built for a scoped ServiceAccount) is the case that prompted the principle.

**Independent Test**: Check three rows, press "Export YAML" in the selection bar, and confirm the dialog shows three documents separated by `---`. Copy and Download both hold all three, and any object that could not be read is listed as an error rather than silently dropped.

**Acceptance Scenarios**:

1. **Given** N checked rows, **When** the operator presses "Export YAML" in the selection bar, **Then** the dialog shows "N objects" and one multi-document YAML in namespace/name order.
2. **Given** some objects cannot be read (deleted meanwhile, or access denied), **When** the export finishes, **Then** the readable objects are exported and each failure is listed with the cluster's own error text.
3. **Given** more than 500 selected objects, **When** the operator exports, **Then** the export is refused with a message to narrow the selection.
4. **Given** the clean toggle, **When** it changes, **Then** every document is re-rendered without re-fetching.

---

### User Story 3 - See the exact manifest before any create or edit is applied (Priority: P2)

Before confirming a create or edit, the operator can see and copy or download the full manifest Fathom is about to send, not just a diff.

**Why this priority**: It closes the other half of the principle (what Fathom creates), but builds on flows that already exist and already show a diff.

**Independent Test**: Fill in the RoleBinding create form, press Review, open "Manifest", and confirm the full YAML is shown with Copy and Download. Copy it and confirm it equals the object that Create sends.

**Acceptance Scenarios**:

1. **Given** the RoleBinding/ClusterRoleBinding form in review, **When** the operator opens the manifest view, **Then** the complete YAML to be applied is shown with Copy and Download.
2. **Given** a YAML edit in review, **When** the diff is shown, **Then** Copy/Download of the edited manifest are available next to it.
3. **Given** a role-assignment preview (OIDC), **When** it is shown, **Then** the rendered binding YAML can be copied and downloaded.
4. **Given** a Helm upgrade or rollback preview, **When** it is shown, **Then** the rendered target manifest can be copied and downloaded.
5. **Given** the kind "Create cluster" confirmation, **When** it opens, **Then** it shows the kind config that will be used (the built-in bare config or the pasted one) with Copy and Download.

---

### User Story 4 - Export a context as a standalone kubeconfig, safely (Priority: P2)

An operator wants a kubeconfig file for the current (or another) context, to hand to kubectl, CI or another tool, without hand-editing `~/.kube/config`. Fathom shows what kind of credential the context uses. It flags embedded secrets, which are not written unless the operator explicitly asks. It writes an owner-only file only after an explicit confirmation.

**Why this priority**: High value, but it handles credentials, so it comes after the read-only manifest paths and needs the strictest posture.

**Independent Test**: Open "Export kubeconfig…" from the context picker, see the redacted preview and the credential type, confirm, and check that a file with exactly one context, one cluster and one user was written with owner-only permissions. For a context with an embedded token, confirm the token is omitted unless "Include embedded credentials" is ticked, and that the token never appears on screen.

**Acceptance Scenarios**:

1. **Given** the context picker, **When** the operator chooses "Export kubeconfig…", **Then** a dialog opens for the current context with a selector for any other context.
2. **Given** a context whose user authenticates through an exec plugin or auth provider, **When** previewed, **Then** the credential is shown as "exec plugin (<command>)" or "auth provider (<name>)" and exported as-is.
3. **Given** a context with an embedded token, client key or password, **When** previewed, **Then** a warning names the embedded credential types, the preview shows `<redacted>` in their place, and "Include embedded credentials" is off.
4. **Given** "Include embedded credentials" is off, **When** exported, **Then** those fields are omitted from the file and the result says so.
5. **Given** the operator presses Export, **When** the confirmation appears, **Then** it names the context, the path, and whether secrets will be written, and nothing is written until confirmed.
6. **Given** the target path exists, **When** exporting without "Overwrite", **Then** the export fails with "file exists" and nothing changes.
7. **Given** the hosted (browser) build, **When** the context picker opens, **Then** no kubeconfig export entry is offered, and the backend refuses the command if sent anyway.
8. **Given** any export, **When** logs and protocol traffic are inspected, **Then** no credential value appears in either.

### Edge Cases

- The object is deleted between selection and export: it is reported as "not found" in the dialog; the others still export.
- Access denied on read (a read-only identity lacking `get` on a kind): reported per object with the API server's message.
- A kind with no API object (synthetic Applications, Helm releases, port forwards, events aggregates): no Export entry is offered for it.
- A custom resource: exported through the same path using the group/version shown in its list.
- A headless Service (`clusterIP: None`): `clusterIP` is kept, because it is user intent, not server-assigned.
- A Job with a generated selector: the selector and its generated template labels are removed so the Job re-applies.
- The Inspector has no raw YAML yet (still loading): Copy/Download are disabled until it arrives.
- A clipboard write is refused by the platform: an error toast says so, and Download still works.
- The kubeconfig's user entry references files (client-certificate, client-key, token-file, CA path): the CA is inlined (it is public), and other file references are kept as paths and listed as "not portable".
- The chosen path is the kubeconfig Fathom itself loaded: refused, so the source is never clobbered.
- The chosen directory does not exist: refused with the OS error.
- A context whose cluster or user entry is missing from the kubeconfig: preview shows the error; export is refused.

## Requirements *(mandatory)*

### Functional Requirements

**Per-resource export (US1)**

- **FR-001**: The Inspector YAML tab MUST offer Copy and Download for the shown object, plus a "Clean for re-apply" toggle, on by default. This holds for both the editable and read-only YAML views.
- **FR-002**: The row menu and the pane header action menu of every list whose kind is exportable MUST offer "Export YAML…", which opens the export dialog for that object. It MUST be offered even when the kind has no mutating actions.
- **FR-003**: The export dialog MUST fetch the objects fresh from the cluster, then show the resulting YAML with a clean toggle (default on), Copy and Download, the object count, and per-object errors and warnings.
- **FR-004**: Export MUST need only read access. Export entry points MUST NOT be gated on write capabilities, and MUST work in hosted mode as the signed-in user.
- **FR-005**: The downloaded filename MUST be `<kind>-<namespace>-<name>.yaml` (lowercased kind, namespace omitted when cluster-scoped) for a single object, and `<kind>-<count>-objects.yaml` for a bundle.

**Clean-for-re-apply rules (US1, US2)**

- **FR-006**: Clean mode MUST remove top-level `status`, and from `metadata`: `managedFields`, `resourceVersion`, `uid`, `creationTimestamp`, `generation`, `selfLink`, `deletionTimestamp`, `deletionGracePeriodSeconds`, and `ownerReferences`.
- **FR-007**: Clean mode MUST remove these server-written annotations: `kubectl.kubernetes.io/last-applied-configuration`, `deployment.kubernetes.io/revision`, `pv.kubernetes.io/bind-completed`, `pv.kubernetes.io/bound-by-controller`, `pv.kubernetes.io/provisioned-by`, `volume.beta.kubernetes.io/storage-provisioner` and `volume.kubernetes.io/storage-provisioner`. It MUST drop `annotations` entirely when nothing is left.
- **FR-008**: Clean mode MUST apply these kind-specific rules. **Service**: remove `spec.clusterIP` and `spec.clusterIPs` unless `clusterIP` is `None`. **Job**: when `spec.manualSelector` is not true, remove `spec.selector` and the generated labels `controller-uid`, `batch.kubernetes.io/controller-uid`, `job-name` and `batch.kubernetes.io/job-name` from `spec.template.metadata.labels`.
- **FR-009**: Clean mode MUST keep everything else, including `name`, `namespace`, `labels`, other annotations, `finalizers` and `spec`. It MUST emit `apiVersion` and `kind` first and `metadata` next.
- **FR-010**: Raw mode MUST output the object as the cluster returned it. The only exceptions are the Secret rules in FR-011.
- **FR-011**: Secret exports MUST replace every `data` and `stringData` value with the placeholder `<redacted>` and MUST always remove the last-applied annotation, in both modes. The dialog MUST state that values were redacted. The Inspector's own Secret view MUST also drop the last-applied annotation.
- **FR-012**: For clean exports of objects that had a controller owner, the dialog MUST warn per object and name the owner kind and name.
- **FR-013**: Generated YAML MUST be valid YAML that round-trips to the same JSON. In particular, strings that look like numbers, booleans, dates or nulls MUST stay strings.

**Bundle export (US2)**

- **FR-014**: The selection bar MUST offer "Export YAML" whenever at least one row is checked in an exportable list.
- **FR-015**: A bundle MUST be one multi-document YAML: documents separated by a `---` line, sorted by namespace then name, each obeying FR-006..FR-013.
- **FR-016**: A bundle MUST be capped at 500 objects. Beyond that the export is refused with a message and nothing is fetched.
- **FR-017**: Objects that fail to read MUST be listed in the dialog with the cluster's error text. They are left out of the YAML, and the YAML is still exportable.

**Manifest before mutation (US3)**

- **FR-018**: The review step of the YAML edit tab, the RoleBinding/ClusterRoleBinding create/edit form, the OIDC role-assignment dialog, the Helm upgrade preview and the Helm rollback preview MUST each offer Copy and Download of the full manifest that will be applied, next to the existing diff.
- **FR-019**: The RoleBinding form review MUST offer a "Manifest" view showing the complete YAML, not only the diff.
- **FR-020**: The kind "Create cluster" confirmation MUST show the exact kind config that will be used, with Copy and Download.

**Kubeconfig export (US4)**

- **FR-021**: The context picker MUST offer "Export kubeconfig…" in desktop builds. It opens a dialog defaulting to the active context, with a selector for any known context.
- **FR-022**: The backend MUST build a standalone kubeconfig holding exactly the chosen context, its cluster and its user entry, with `current-context` set to it.
- **FR-023**: The backend MUST classify the user credential as: exec plugin, auth provider, bearer token, token file, client certificate (data or file), basic auth, or none. It MUST report which embedded secret fields are present: token, client-key-data, password, and auth-provider secret keys (`id-token`, `refresh-token`, `access-token`, `client-secret`).
- **FR-024**: Exec plugin entries MUST be exported as-is (command, args, env, apiVersion). Auth-provider entries MUST be exported as-is except for the secret keys in FR-023, which follow FR-025. The backend MUST NOT materialize a token by running an exec plugin or refreshing an auth provider.
- **FR-025**: Embedded secret fields MUST be omitted from the written file unless the operator explicitly ticks "Include embedded credentials". That option is off by default and only offered when such fields exist.
- **FR-026**: The preview sent to the interface MUST have every secret field replaced by `<redacted>`. The unredacted kubeconfig MUST never be sent over the protocol, rendered, or copied to the clipboard.
- **FR-027**: A CA given as a file path MUST be inlined as data when readable. Client certificate, client key and token file paths MUST be kept as paths and listed as "not portable" in the preview.
- **FR-028**: Writing MUST need a separate explicit confirmation naming the context, the target path, and whether embedded credentials are included.
- **FR-029**: The file MUST be created with owner-only permissions (0600 on Unix). An existing file MUST NOT be replaced unless "Overwrite" is ticked, and even then MUST be left with 0600.
- **FR-030**: The export MUST refuse the kubeconfig file Fathom loaded contexts from, relative paths after `~` expansion, and missing parent directories.
- **FR-031**: The backend MUST refuse kubeconfig preview and export when running as the hosted server. The hosted UI MUST NOT show the entry point.
- **FR-032**: Credential values MUST NOT be written to logs, the audit log, notifications or errors. Logs MAY name the context and the path.

### Key Entities

- **Manifest export request**: a list of object references (apiVersion, kind, namespace, name) plus a client request id. Its reply holds the raw objects that could be read and a per-object error list.
- **Export document**: one object rendered as YAML under a mode (clean or raw), with any warnings (owner references, redacted Secret).
- **Kubeconfig export preview**: the context name, the redacted YAML, the credential classification, the embedded secret fields present, the non-portable file references, warnings and a suggested path.
- **Kubeconfig export result**: the path written and whether embedded credentials were included, or an error.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A cleaned single-object export of a live Deployment is accepted by a server-side dry-run apply against the same cluster, with no hand edits.
- **SC-002**: From any exportable object, getting its YAML onto the clipboard takes at most 2 interactions from the Inspector (tab, Copy), or 3 from a list row (menu, Export YAML, Copy).
- **SC-003**: A 50-object bundle export shows its YAML within 3 seconds on a local cluster.
- **SC-004**: 100% of the create/edit flows listed in FR-018 and FR-020 offer Copy and Download of the manifest before the confirm step.
- **SC-005**: In automated tests, no credential value from a test kubeconfig appears in any protocol message, rendered text, clipboard write or log line, and the written file has owner-only permissions.
- **SC-006**: A read-only identity can use every export entry point that a read-write identity can.

## Assumptions

- Only kinds with a known API group/version are exportable: every built-in kind Fathom lists, plus custom resources through their list's group/version. Synthetic views (Applications, Helm releases, port forwards, overview panels) are not.
- The Inspector copies from the raw object it already holds (kept current by its watch). The export dialog always re-reads from the cluster.
- Downloads use the same browser download path Fathom already uses for the OIDC config file.
- Copying a clean manifest from the Inspector does not stop the operator from editing in the YAML tab. Export and edit are independent.
- Namespace-wide and Application-wide bundle export, zip bundles, scoped ServiceAccount kubeconfig minting, kubeconfig merge, Helm chart/kustomize generation, OIDC-to-kubeconfig sync, and manifest views for node shell/debug containers stay in the backlog.
