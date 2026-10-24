# Files in volumes

The **Files** tab of a stack lets you look into the folders its services keep their data in: a game
server's world saves and `adminlist.txt`, a Minecraft `server.properties`, a web app's `config.yml`.
You can read and edit text files in a code editor, download files and folders (for a backup),
upload files (for a restore), and undo a bad save from the file's history, all without a shell on
the host.

## Who can use it

- **Anyone who may edit the stack** sees the tab. People who may only view a stack get no Files tab,
  and the server refuses their file requests too. Files often hold passwords that are not masked
  the way `.env` values are, which is why reading them needs the same permission as editing.
- **On a network Depths does not trust yet** (the [restricted view](security.md)) you can list
  folders but not open, download, upload or change anything.
- **The operator decides how much is on.** `DEPTHS_FEATURE_FILES` is `full` by default. With
  `read_only` you can browse, open and download, and every control that changes something is
  hidden. With `off` there is no tab. See [Settings](#settings).

Every open, download, upload, save, create, rename, delete and restore is recorded in the audit log
with who, when, stack, service, mount, path and size, and so is every refusal. File contents are
never written to the log.

## Opening the Files tab

Open a stack and choose **Files**. Depths lists every mount of every service in the stack:

- the **source**: a volume's name, or the host folder of a bind mount;
- the **kind**: named volume or bind mount;
- the **path inside the container**, for example `/config`;
- whether it is **read-only** in the container.

A mount that several services share is listed under each of them. It counts as writable when at
least one service mounts it writable.

If the stack has never been deployed, Depths lists the mounts the compose file declares and says
that named volumes only appear once the stack has been started. A relative bind source is resolved
against the stack's directory, the way Compose does it. A source Depths cannot work out (for
example, from an unset variable) is listed as unresolved rather than guessed.

Choose a mount to open it. Folders come first, then files, each with its size, modification time
and its owner and permissions in a compact form. Walk into a folder by choosing it; you can go back
up to the mount's root and never above it. The filter box narrows a long folder by name. The
address in the browser names the stack, the mount and the path, so you can bookmark a folder or
send the link to someone else who can edit the stack.

On a phone the listing and the open file take turns; on a wide screen they sit side by side.

### What cannot be browsed

Some mounts are listed with a reason instead of a folder to open:

- **Temporary (tmpfs) mounts** live in a container's memory; there is nothing on disk to look at.
- **Bind mounts of sensitive host paths**: the host's `/`, `/etc`, `/proc`, the Docker socket and
  the other places [Containers and shells](security.md#containers-and-shells) rates as root or
  escape. The explorer is not a way into the host's own files.
- **A single-file or socket bind**, which has no folder to list.
- **A named volume that does not exist yet**, or one that was removed.

Inside a mount you can also meet:

- **Links that stay inside the mount** are followed. A link that points outside the mount, or an
  absolute link, is shown with its target and is never followed, opened, written through or
  downloaded through.
- **Files with more than one hard link** open read-only. Writing one would also change its other
  name, which might be outside the volume.
- **Device files, sockets and pipes** are listed as such and never opened.

## Reading and editing a file

Open a text file and it appears in the editor with line numbers, highlighting chosen from the file's
name or content (INI and CFG, JSON, YAML, TOML, XML, properties, shell scripts, Lua and plain
text), search and replace, undo and redo, and soft wrap. Everything works from the keyboard.

A file opens in the editor when it is valid UTF-8 text and no bigger than the editor limit (2 MiB
by default). Anything else, a binary file or a text file over the limit, shows its size and type and
offers a download instead.

The editor is read-only, and says why, when the mount is read-only in every container that uses it,
when the file has more than one hard link, or when the operator has set
`DEPTHS_FEATURE_FILES=read_only`.

Depths keeps the file's **line endings** (LF or CRLF), its **trailing newline** and a leading
**byte-order mark** as they were, unless you changed them. A file that mixes line endings opens
with a notice that saving will use the most common one.

### Format checks

For JSON, YAML, TOML, XML and INI-style files the editor marks a syntax error on its line as you
type. It only warns: you can still save after a confirmation. Depths never refuses a save because
of a file's format.

## Saving

Choose **Save**. Depths first shows what will change, the way it does for a compose file, and writes
nothing until you confirm.

A save is all or nothing: the file holds either its old content or the new one, never a mix, even
if the disk fills up half way. Depths writes a new file next to the old one and swaps it in, so the
file keeps its **owner, group and permission bits** and the game server can still read it.

Depths acts as the file's owner, not as an all-powerful user. A file its owner could not write
cannot be written here either, and the error says so in the system's own words.

**What a save does not keep.** Extended attributes and access control lists (ACLs) are not copied
to the new file. They are rare on game and home server volumes. If your files use them, put them
back by hand after saving.

### When the file changed underneath you

If the file changed on disk after you opened it (a server rewrote it, or somebody else saved it),
the save is refused. You can:

- **Reload** the file, which drops your edit; or
- **Compare** your version with the one on disk, and then save over it on purpose.

Depths decides this by comparing a fingerprint of the content taken when you opened the file with
the file's content just before the swap. There is a tiny gap, microseconds, between that last
check and the swap itself. A change landing inside that gap would be overwritten, but the history
(below) still holds what was replaced.

### Save and restart

Game servers often write their configuration back out when they stop, so a change saved while the
server runs may be overwritten or ignored until it restarts. When a running container mounts the
file's mount, Depths says so and offers:

- **Save**, and leave the service alone; or
- **Save and restart `<service>`**, which saves and then restarts that one service as an ordinary
  restart, with its own confirmation and audit entry. This is offered only if you may restart the
  stack.

If you have unsaved changes and try to leave the file or the page, Depths asks first.

## Downloading

Choose **Download** on a file to get it byte for byte. Choose it on a folder to get one `.tar`
archive of the folder with its structure; it is not compressed, because most game saves already are.
Downloads are streamed, so a large one does not fill Depths' memory.

The download limit is 2 GiB by default. For a folder, Depths measures it first; above the limit it
tells you how big the folder is and does not start.

Download a world folder before a risky change. It is the cheapest backup there is.

## Uploading

Choose **Upload** in a writable folder and pick one or more files. Depths then asks you to confirm,
naming the folder and every file. A name that already exists is marked as a **replacement**, so you
cannot overwrite a file by accident.

Each file is limited to 1 GiB by default and lands all or nothing. A file that replaces another
keeps the old file's owner, group and permissions; a new file takes the owner and group of the
folder it lands in.

If a service is using the mount and you are replacing a file, Depths warns that the server may hold
the file open or overwrite it, and offers to stop the service first. For a world save that is what
you want: stop the server, upload the older world, start it again.

## Creating, renaming and deleting

In a writable folder you can create a file or a folder, rename or move an entry within the same
mount, and delete files and folders. Moving between mounts is not offered.

- New entries take the **owner and group of the folder** they are created in.
- A rename whose two folders have different owners is refused, because no single user could do it
  without extra privileges.
- **Delete** asks you to confirm, names what will go and, for a folder, says how many entries it
  holds. If it reaches something its owner cannot remove, it stops and tells you what was removed
  and the system's error, so nothing is hidden.

## History

Before Depths overwrites, replaces or deletes a file, it keeps the previous content, with the time
and who did it. Open a file's **History** to see the kept versions, newest first. You can compare
any version with the current file and **restore** it. A restore is itself a save, with its own
history, so you can undo an undo. A deleted file can be restored the same way while its history is
kept.

History is a safety net for mistakes made in the explorer, not a backup system:

- **Where it lives.** In Depths' data volume (`/var/lib/depths/file-history`), not in the volume you
  edited. Back it up with the rest of that volume, and leave it room.
- **Limits.** Files over 10 MiB get no history. Depths keeps 20 versions per file, 1 GiB in total
  and 30 days. When a limit is reached the oldest versions go first.
- **What it does not see.** Changes made by the service itself, by a shell, or by anything other
  than the explorer are not recorded.

## Settings

The operator sets these in `.env`. A value that does not parse stops start-up with a message
naming it. Sizes take `KiB`, `MiB` and `GiB`.

| Environment | Default | What it does |
|---|---|---|
| `DEPTHS_FEATURE_FILES` | `full` | `off` hides the tab, `read_only` allows browse, open and download, `full` allows everything. Read at start-up |
| `DEPTHS_HELPER_IMAGE` | Depths' own image | The image the volume helper runs from (below) |
| `DEPTHS_FILES_EDITOR_MAX` | `2MiB` | Largest file opened in the editor |
| `DEPTHS_FILES_DOWNLOAD_MAX` | `2GiB` | Largest file or folder download |
| `DEPTHS_FILES_UPLOAD_MAX` | `1GiB` | Largest upload, per file |
| `DEPTHS_FILES_HISTORY_FILE_MAX` | `10MiB` | Larger files get no history |
| `DEPTHS_FILES_HISTORY_VERSIONS` | `20` | Versions kept per file |
| `DEPTHS_FILES_HISTORY_TOTAL` | `1GiB` | Total size of all history |
| `DEPTHS_FILES_HISTORY_DAYS` | `30` | Versions older than this are dropped |
| `DEPTHS_FILES_HELPERS` | `8` | Most volume helpers running at once |

## How Depths reaches a volume

Depths itself never mounts your volumes, and the example compose file needs no new mount, proxy
switch or capability for this. For each mount you open, Depths starts a short-lived **helper
container** from its own image, through the same Docker socket proxy it already uses. The helper:

- sees exactly one mount, and nothing else on the host;
- has no network, a read-only root filesystem and no capabilities, and cannot gain any;
- runs as the owner of the file or folder it is working on, with small memory, process and CPU
  limits;
- opens every path beneath the mount's root with the kernel's own containment (`openat2`), so a
  `..`, an absolute path, or a link swapped at the last moment cannot reach outside it.

A helper stays around for about 90 seconds after you stop using a mount, so browsing is quick, and
is then removed. If Depths stops unexpectedly, it removes any helpers left over the next time it
starts. Helpers carry the label `depths.helper=1`, which is how you can recognise them in
`docker ps`.

This needs:

- **Linux kernel 5.6 or newer** on the Docker host (for `openat2`). On an older kernel, or where a
  seccomp profile blocks `openat2`, the Files tab turns itself off for that run.
- **A helper image.** By default Depths finds its own image through its own container and uses
  that. If it cannot, the tab is off until you set `DEPTHS_HELPER_IMAGE` to the image ID
  (`sha256:...`) of a Depths image of the same version. The helper always runs by image ID, never
  by tag, and Docker never pulls it.
- **A socket proxy that lets Depths create containers**, which the example compose file already
  allows. If it is denied, every helper start fails.

When the tab is off because of one of these, admins are shown the reason.

## If something does not work

| You see | Why | What to do |
|---|---|---|
| No Files tab | You may only view the stack, or the operator set `DEPTHS_FEATURE_FILES=off` | Ask an admin for edit access, or check the setting |
| No tab for anyone, and admins see a reason | No helper image, or the kernel is older than 5.6 | Set `DEPTHS_HELPER_IMAGE`, or update the kernel |
| *Not created yet* on a mount | The named volume does not exist | Deploy the stack |
| *Changed on disk* when saving | Someone or something changed the file after you opened it | Reload, or compare and save over it |
| *Permission denied* | The file's owner cannot write it | Fix the ownership on the host; Depths will not override it |
| Folders list but files will not open | You are on a network Depths does not trust yet | See [Networks and the restricted view](security.md) |
