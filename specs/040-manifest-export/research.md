# Research: 040 Manifest Export

## R1. Where do the clean rules run: backend or UI?

- **Decision**: In the UI (`ui/src/features/manifest-export/clean.ts`). The backend returns raw JSON.
- **Rationale**: The Inspector already holds the object's `raw_json`, so Copy there needs no round trip. Toggling clean on and off re-renders without re-fetching (US2-4). Mock-mode e2e exercises the real rule code, not a fake copy of it. Cleaning is presentation of data the UI already received; it touches no cluster or credential, so Principle II holds.
- **Alternatives**: Rust-side cleaning returning YAML. That would duplicate the rules in the fake transport, and every toggle would need a new fetch. Rejected.

## R2. YAML serializer

- **Decision**: Add the `yaml` package (eemeli/yaml v2, zero dependencies) for export output. Leave the existing hand-rolled `resources/yaml.ts` for the Inspector display so this feature does not churn its snapshots.
- **Rationale**: Re-apply correctness (FR-013) needs spec-correct quoting. The hand-rolled emitter leaves some strings plain that YAML resolvers then retype or reject: strings starting with `@`, `0x1F`, `.inf`, date-like strings, `#`-containing values. `yaml` stringifies with core-schema quoting and parses back identically.
- **Alternatives**: `js-yaml` (only a transitive dependency; its dump is fine but it is larger). Fixing the hand-rolled emitter (a correctness rabbit hole).

## R3. Fetching arbitrary objects

- **Decision**: `FetchManifests { request_id, objects: [{api_version, kind, namespace, name}] }`. The backend runs `discovery::oneshot::pinned_kind` once per distinct GVK, then `Api<DynamicObject>::get`, 8 at a time. It returns `FetchedManifest { index, raw_json }` and `ManifestFetchError { index, message }`.
- **Rationale**: This is the same discovery approach as `ApplyResourceYaml`. It covers built-ins and CRDs without a per-kind table in Rust, and the request id lets the dialog ignore stale replies.
- **apiVersion source**: the UI table `kinds.ts` maps each built-in kind Fathom lists to its apiVersion. CRD instance lists supply group/version from their route. Kinds absent from the table are not exportable.

## R4. Secret handling

- **Decision**: For `kind == Secret`, the backend replaces every `data`/`stringData` value with `<redacted>` (the literal string, left un-base64'd so a re-apply fails validation) and always drops `kubectl.kubernetes.io/last-applied-configuration`. The existing Inspector `redacted_raw_json` also drops that annotation.
- **Rationale**: Principle VI. The last-applied annotation of a Secret applied with `kubectl apply` contains the full plaintext manifest, and today it reaches the Inspector. This fixes that.

## R5. Kubeconfig handling: typed or untyped?

- **Decision**: Untyped. Parse the kubeconfig file Fathom loaded (`backend::kube::kubeconfig_path()`) into `serde_json::Value` via `serde_yaml`. Pick the named context, cluster and user entries verbatim, then emit `apiVersion: v1`, `kind: Config`, `clusters`/`contexts`/`users` with one entry each, and `current-context`.
- **Rationale**: kube-rs's typed `Kubeconfig` round-trips only known fields. Untyped copying keeps exec (`interactiveMode`, `provideClusterInfo`, `installHint`), `extensions`, `proxy-url` and similar fields exactly as-is, which the "exec/auth-provider as-is" requirement asks for.
- **Credential classification** (the user entry), in precedence order: `exec` → Exec{command}; `auth-provider` → AuthProvider{name}; `token` → Token; `tokenFile` → TokenFile; `client-certificate(-data)` → ClientCertificate; `username`+`password` → Basic; else None.
- **Embedded secret fields**: `token`, `client-key-data`, `password`, and the `auth-provider.config` keys `id-token`, `refresh-token`, `access-token` and `client-secret`. Unless the operator includes them, they are removed from the written file. In the preview they are always replaced by `<redacted>`.
- **File references**: a `certificate-authority` path is inlined as `certificate-authority-data` (base64) when readable, since a CA is public. `client-certificate`, `client-key` and `tokenFile` are kept as paths and reported as not portable.
- **No materialization**: exec plugins are never run and auth providers are never refreshed.

## R6. Writing the file

- **Decision**: The backend writes it. `OpenOptions::new().write(true).create_new(!overwrite).create(true).truncate(true).mode(0o600)` on Unix, then `set_permissions(0o600)` in case an existing file had looser bits. The path is expanded from `~`; relative paths, a missing parent, and the source kubeconfig (canonicalized comparison) are refused.
- **Rationale**: A browser download cannot set 0600, and would require sending the secret over the protocol. The default path is `~/.kube/fathom-<sanitized-context>.yaml`, next to the operator's other kubeconfigs.
- **Windows**: there is no Unix mode. The file is created in the user profile by default, which inherits user-only ACLs. This is documented.

## R7. Download and copy in the webview

- **Decision**: Reuse the existing pattern (`Blob` + `URL.createObjectURL` + `<a download>`, as in `resources/oidcConfig.tsx`) and `navigator.clipboard.writeText`, with toasts on success or failure.
- **Rationale**: This already ships in Tauri and browser builds. Playwright can assert downloads via `page.waitForEvent('download')`, and clipboard via the `prepareClipboard`/`readClipboard` helpers (WebKit shim).

## R8. Where the UI entry points go (merge-conflict budget)

- The Inspector gets one additive line: `<InspectorYamlExportBar …/>` at the top of the YAML `TabsContent`.
- In `ActionMenu.tsx`, the export item is appended after `ActionItems` in both menus, and the selection bar gets an Export button. The early `return null` guards are widened to "has actions OR exportable", so read-only kinds still get the menu.
- The dialog host is mounted inside `ActionDialogHost.tsx` (one line), avoiding `App.tsx`/nav edits.
- `ContextPicker.tsx` gets one menu entry.
