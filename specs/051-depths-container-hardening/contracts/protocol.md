# Protocol additions (051)

- `ContainerSummary.security: { level, reasons: Reason[] }`; `StackSummary.security_level`.
- `Event::Inventory` carries `daemon_mode`; admins get `rootful_daemon_notice: bool`.
- `Cmd::OpenShell` unchanged on the wire; refusal kinds added: `shell_escape`, `shell_root_admin_only`, `shell_unverifiable`.
- `Cmd::LintCompose { stack, body }` → `Event::LintResult { findings[] }`; `Cmd::SaveFile`/`Cmd::Up` gain `acknowledge_findings: bool` (ignored unless admin and policy enforce); refusal kind `compose_blocked { findings[] }`.
- Shells closed on recreate emit `Event::ShellEnded { reason: "container_recreated" }`.
- Bindings regenerated with `cargo test -p depths-protocol --test export_bindings`.
