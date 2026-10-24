# User guides

Short, task-shaped guides for the two apps in this repository.

## Fathom

For Kubernetes, on the desktop or hosted in a cluster. Every screenshot in these guides was taken against a real
k3s cluster, through a read-only service account scoped to the `fathom-dev` namespace.

| Guide | What it covers |
|---|---|
| [Getting started](getting-started.md) | Install, pick a context, find your way around the layout |
| [Inspecting resources](inspecting-resources.md) | The cascading columns, the inspector tabs, topology, images |
| [Workloads: logs, scale, port forwards](workloads.md) | Day-to-day operations on pods and deployments |
| [Applications](applications.md) | Grouping workloads into applications and drilling into them |
| [Access control](access-control.md) | Browsing and editing RBAC, OIDC sign-in and role groups |
| [AI Assist](ai-assist.md) | Asking about a resource, bringing your own key, staying offline |
| [Settings and teams](settings-and-teams.md) | Per-cluster settings, app settings, shared connection profiles |
| [Phone and tablet](phone-and-tablet.md) | How the layout adapts to small screens |
| [Keyboard shortcuts](keyboard-shortcuts.md) | Every binding |
| [Verifying a download](verifying-downloads.md) | Checksums and build provenance for release files |

## Depths

For Docker Compose stacks on one host, from a browser. Start with the
[Depths guides](depths/README.md).

| Guide | What it covers |
|---|---|
| [Installing Depths](depths/install.md) | The example compose file, the socket proxy, the reverse proxy, every setting |
| [Sign-in providers](depths/sign-in.md) | OIDC, a GitHub OAuth App or a Discord application |
| [People and access](depths/users.md) | The owner, finding your id, admitting people, assigning stacks |
| [Migrating from Dockge](depths/migrating-from-dockge.md) | Pointing Depths at Dockge's directory, what differs, switching back |
| [Tabs, logs and shells](depths/tabs-and-shells.md) | Logs, stats and shell tabs, panes, dropped connections |
