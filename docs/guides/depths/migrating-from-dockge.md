# Migrating from Dockge

Depths reads the same stacks directory Dockge does, runs Compose the same way, and keeps its own
state somewhere else. You can run both against the same directory while you try Depths, and go
back to Dockge at any time.

## Point Depths at Dockge's directory

Dockge's stacks directory is usually `/opt/stacks`, which is also Depths' default. If yours is
elsewhere, set `STACKS_DIR` in Depths' `.env` to the same path (see [Installing Depths](install.md)).
Depths also reads Dockge's own variable name, `DOCKGE_STACKS_DIR`, if you reuse a Dockge `.env`.

Mount it at the same path inside the container as on the host, exactly as Dockge requires. The
example compose file already does this. If the paths differ, relative bind mounts in your stacks
resolve to the wrong place, and Depths shows a warning on the stacks it finds started from another
path.

Depths listens on port 8080, not Dockge's 5001, so both can run side by side.

## Permissions on the stacks directory

Dockge runs as root. The Depths container runs as user 10001, so it can only change what that user
may write. To save edits and create stacks, user 10001 needs write access to the stacks directory
and to each stack directory and file you want to edit through Depths. For example, with ACLs:

```sh
sudo setfacl -R -m u:10001:rwX /opt/stacks
sudo setfacl -R -d -m u:10001:rwX /opt/stacks
```

Reading, starting and stopping stacks need only read access, because Compose runs against the
Docker API, not the files.

## What Depths does the same way

- **A stack is a sub-directory with a compose file.** Only the immediate sub-directories count, as
  in Dockge. The directory name is the stack name and the Compose project name.
- **Commands run inside the stack directory, with no `-f`.** So the project Depths starts is the
  same project Dockge and `docker compose` in a terminal see. Starting a stack runs
  `docker compose up -d --remove-orphans`; stop, restart, down and pull are the plain Compose
  commands.
- **`global.env`.** If `<stacks>/global.env` exists, Depths passes it to every command as
  `--env-file ../global.env`, followed by `--env-file ./.env` when the stack has a `.env`. That is
  what Dockge's development branch does. The second flag is needed because any explicit
  `--env-file` turns off Compose's automatic `.env`. With no `global.env`, Depths passes no
  `--env-file` at all, as Dockge 1.5.0 does.

## What Depths does differently

- **Which compose file wins.** When a directory holds more than one compose file, Dockge and
  Compose disagree about which one is the stack. Compose uses the first of `compose.yaml`,
  `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`; Dockge looks in the order
  `compose.yaml`, `docker-compose.yaml`, `docker-compose.yml`, `compose.yml`. So with both
  `docker-compose.yml` and `docker-compose.yaml`, Dockge can show and edit a file Compose never
  runs. Depths always picks the file Compose picks, and shows a warning naming both files and the
  one in use. The fix is to delete or rename the extra file.
- **Invalid names are flagged.** A directory whose name is not a valid Compose project name
  (lowercase letters, digits, `-` and `_`, starting with a letter or digit) is listed and marked
  invalid, and Depths will not act on it. Dockge shows such stacks as inactive.
- **Files are saved exactly as typed.** Depths has no visual compose editor, so it never rebuilds
  your YAML or drops comments.
- **Delete keeps the directory by default.** Deleting a stack runs `docker compose down
  --remove-orphans`; removing the directory is a separate choice you have to tick. Dockge removes
  the directory.
- **Projects outside the stacks directory** (started from elsewhere with `docker compose`) are
  shown to admins only, and are read-only.
- **Sign-in and access.** Dockge has one local account. Depths signs people in through OIDC,
  GitHub or Discord and gives each person access to chosen stacks. See
  [Sign-in providers](sign-in.md) and [People and access](users.md).

## What Depths writes, and what it does not

In the stacks directory, Depths writes only:

- a stack's compose file and its `.env`, when you save them in the editor;
- a new stack directory with those two files, when an admin creates a stack;
- short-lived temporary files named `.depths-tmp-*` while saving, which are renamed over the file
  being saved. A save either lands whole or not at all.

A new `.env` is created with mode 0600. An existing file keeps its mode.

Depths never touches Dockge's own data (its SQLite database and settings in Dockge's data
directory), never writes `global.env`, and never deletes a stack directory unless an admin ticks
that option. Its own state, the audit log and `users.json`, lives in its data directory
(`/var/lib/depths`), outside the stacks directory.

## Switching back

Because Depths leaves the stacks in Dockge's format and keeps nothing of its own beside them,
switching back is just stopping Depths:

```sh
docker compose down      # in Depths' own directory, not a stack's
```

Your stacks keep running: stopping Depths does not stop them. Start Dockge again (if you stopped
it) and it sees the same stacks, including any edits you made in Depths.

Two things to check after switching back:

- If you created a stack directory with more than one compose file while using Depths, Dockge may
  pick a different one (see above).
- If Depths' edits left files owned by user 10001, Dockge, running as root, can still read and
  write them.
