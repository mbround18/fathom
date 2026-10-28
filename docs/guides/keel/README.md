# Keel user guides

Keel runs Ansible against your hosts from a browser. Each workspace binds a repository of playbooks
and inventories, hosts enrol with a one-time token and connect back to Keel, and runs are reviewed
before they change anything.

| Guide | What it covers |
|---|---|
| [The enrolment snippet](enrollment-template.md) | The command shown when a host is enrolled: placeholders, the default and examples |
| [Instance roles from your identity provider](oidc-roles.md) | Mapping OIDC claims to owner, admin and user, and what owners see |
| [The System workspace](system-workspace.md) | Letting the instance owner run Ansible against Keel's own host |
