# Keel user guides

Keel runs Ansible against your hosts from a browser. Each workspace binds a repository of playbooks
and inventories, hosts enrol with a one-time token and connect back to Keel, and runs are reviewed
before they change anything.

New to Keel? Start with [Getting started](getting-started.md).

| Guide | What it covers |
|---|---|
| [Getting started](getting-started.md) | From an empty Keel to the first run: workspace, access, host, receiver, binding |
| [Hosts and enrolment](hosts.md) | Enrolling a host, checking the host CA fingerprint and running the base profile |
| [Receivers](receivers.md) | Creating a receiver, adding its webhook at your Git host, and reading deliveries |
| [Bindings and credentials](bindings.md) | Repository credentials and binding a branch |
| [Runs and approval](runs.md) | Run states, approval policies, approving or denying a run, four-eyes and expiry |
| [Reviews and trusted signers](reviews.md) | The early check, trusted signers, and approving or denying a change |
| [Inventory](inventory.md) | Managed and repository inventory |
| [The enrolment snippet](enrollment-template.md) | The command shown when a host is enrolled: placeholders, the default and examples |
| [Instance roles from your identity provider](oidc-roles.md) | Mapping OIDC claims to owner, admin and user, and what owners see |
| [The System workspace](system-workspace.md) | Letting the instance owner run Ansible against Keel's own host |
| [Keeping Keel up to date](updates.md) | Which build runs, update checks, updating from the card or by hand, and rolling back |
