# Migrating from Watchtower

If your services carry Watchtower labels, you don't have to change them. Depths reads the same
labels and honours them, so a service you told Watchtower to leave alone is left alone by Depths
too. What differs is that Depths shows you every update first, and only updates on its own the
stacks an admin set to **Auto** (see [Image updates](updates.md)).

Depths never edits your compose files or labels.

## The labels

Depths reads the labels of the running container, which include those inherited from the image, as
Watchtower does. For a service with no running container, it reads the labels in the compose file.

All labels start with `com.centurylinklabs.watchtower.`.

| Label | What Watchtower does | What Depths does |
|---|---|---|
| `enable=false` | Never updates the container | Excludes the service |
| `enable=true` | With `--label-enable`, only these are updated | With **Only labelled services** on, only these are checked and updated |
| `monitor-only=true` | Checks and notifies, never updates | Checks and shows, never updates on its own: the policy is at most **Notify** |
| `scope=<name>` | With `--scope <name>`, only these containers | With the **Scope** setting, only these services. Scope `none` matches services with no scope label |
| `no-pull=true` | Never pulls, only restarts on a locally changed image | Excludes the service: Depths only updates by pulling |
| `lifecycle.pre-check`, `post-check`, `pre-update`, `post-update` (and their `-timeout`) | Runs the command in the container | **Not run**, shown on the service with the reason |
| `depends-on=<names>` | Restarts the listed containers in order | **Not run**: the order comes from the compose file |
| `stop-signal=<signal>` | Stops with that signal | **Not used**: compose's `stop_signal` applies |

A label can only make a service stricter than its stack's policy, never looser. A stack set to
**Off** stays off whatever its labels say, and a `monitor-only` service on an **Auto** stack is only
shown. The Updates page names the label that decided.

True and false are read as Watchtower reads them: `true`, `True`, `TRUE`, `t`, `T`, `1` and their
false forms. Anything else, such as `yes` or `on`, is ignored and shown as unrecognised. Scopes are
case-sensitive.

**Only labelled services** and **Scope** are under **Settings** on the Updates page. They are the
equivalents of Watchtower's `--label-enable` and `--scope`.

## What Depths doesn't run

Lifecycle hooks are commands that run inside the container. Depths only runs commands in a
container through its guarded shell (see [Containers and shells](security.md#containers-and-shells)),
so it never runs a hook. If a service relies on a pre-update hook, for example a backup, keep its
stack on **Notify** and run the step yourself before you update.

## Running both at once

Two updaters on one host would race each other. When Depths sees a running Watchtower container,
recognised by the `com.centurylinklabs.watchtower=true` label or by its image (Watchtower and its
common forks), it:

- shows a warning on the Updates page naming the container;
- pauses every **Auto** update on the host. Checks, and updates you confirm by hand, go on.

Depths never stops, restarts or relabels the Watchtower container. Choose one of these, then
answer the warning on the Updates page (an admin, with a passkey; recorded in the audit log):

- **I stopped it.** Stop and remove Watchtower, for example by deleting its service from its
  compose file and running `docker compose up -d --remove-orphans` there. Then answer **Stopped**.
  If it comes back, Depths warns again.
- **It only monitors.** Keep Watchtower for notifications, run with `--monitor-only` (or
  `WATCHTOWER_MONITOR_ONLY=true`), so it never updates anything. Then answer **Monitor only**.
  Depths resumes Auto updates and leaves Watchtower running.

The answer is for that container on that image. A new Watchtower container, or the same one
recreated on a newer image, gets a new warning.

## Moving a service over

1. Start Depths next to Watchtower. Auto updates stay paused, so nothing changes yet.
2. Turn checks on (see [Turning checks on](updates.md#turning-checks-on)) and look at what Depths
   finds. Your labels already apply.
3. Set the stacks Watchtower updated for you to **Auto**, with a maintenance window.
4. Stop Watchtower, or set it to monitor-only, and answer the warning.
