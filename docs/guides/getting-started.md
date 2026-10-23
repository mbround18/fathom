# Getting started

## Install

Download a bundle from the [Releases page](https://github.com/mbround18/fathom/releases):

- **Debian / Ubuntu:** `sudo apt install ./Fathom_<version>_amd64.deb`
- **Fedora / RHEL:** `sudo dnf install ./Fathom-<version>-1.x86_64.rpm`
- **Any distro:** `chmod +x Fathom_<version>_amd64.AppImage && ./Fathom_<version>_amd64.AppImage`

Bundles are not code-signed. [Verifying a download](verifying-downloads.md) explains how to check
one before you run it.

## Pick a context

Fathom reads your kubeconfig (`~/.kube/config`, or whatever `KUBECONFIG` points at) and never
changes it. If the kubeconfig has no current context, the first screen asks you to pick one. Fathom
remembers your choice for next time. From the same screen you can reload the kubeconfig, or create
a local [kind](https://kind.sigs.k8s.io/) cluster to experiment on.

Switch contexts later with the **CTX** picker in the top bar. The **NS** picker next to it filters
every view to one namespace.

## The layout

![Pods in fathom-dev, with one pod open in the inspector](images/wide-inspector.png)

The wide layout has four columns, read left to right:

1. **Sections**: everything Fathom can show, grouped as Cluster, Workloads, Networking, Storage,
   Config, Access control, Custom resources, Integrations, Containers and Settings.
2. **The list** for the section you picked, with a filter box (`/` focuses it).
3. **The inspector** for the row you opened.
4. **Next level**: follow a link in the inspector (an owner, a node, a related object) and it opens
   here, without losing your place.

The dock along the bottom streams logs and events. The status bar at the very bottom shows the
connection state and the active context.

Fathom only offers what your identity can do. If your RBAC doesn't allow a verb, its button is
disabled, not hidden, so you can see what exists.

## Next

- [Inspecting resources](inspecting-resources.md)
- [Keyboard shortcuts](keyboard-shortcuts.md)
