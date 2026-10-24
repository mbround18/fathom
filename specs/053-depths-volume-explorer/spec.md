# Feature Specification: Depths volume explorer

**Feature Branch**: `050-depths-stack-manager` (spec 053 builds on 050, 051 and 052 and lands on the same branch)

**Created**: 2026-10-24

**Status**: Draft

**Input**: User description: "for depths i want to have a volume explorer letting the uwser excpore the volume of a service like valheim saves or server dir to beable to change ocnfiguration weitha nice code editor."

## Overview

A stack's compose file is only half of a running service. The rest lives in the service's
volumes: a Valheim server's world saves and its `adminlist.txt`, a Minecraft server's
`server.properties`, a web app's `config.yml`. Today, changing any of those means a shell on the
host. Depths will let a person who may edit a stack browse the volumes its services mount, read
and edit the configuration files in them with a proper code editor, and download or upload files
(for example, to back up or restore a world save). It does this without giving Depths, or the
person, any reach outside the volume they chose.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Browse a service's volumes and read a file (Priority: P1)

A person opens the `valheim` stack and picks the **Files** tab. They see each service's mounts:
the named volume `valheim-config` at `/config`, and the bind mount `./server` at `/opt/valheim`.
Each mount shows its type, its path inside the container, and whether it is mounted read-only.
They open `/config`, go into `worlds_local`, and see the world files with their size and
modification time. They open `adminlist.txt` and read it in the editor, with line numbers and
syntax highlighting.

**Why this priority**: Seeing what is in a volume is the base for everything else, and on its
own it already replaces most "ssh in and look" sessions.

**Independent Test**: Seed a stack whose service mounts a named volume and a bind mount, each
holding text and binary files and a subfolder. Open the Files tab, walk into the subfolder, and
open a text file. The listing and the file's contents match what is on disk. A binary file shows
its size and type, and offers a download instead of the editor.

**Acceptance Scenarios**:

1. **Given** a stack whose services mount volumes, **When** a person with edit permission on it opens the Files tab, **Then** they see every mount of every service: name or host source, type (named volume or bind), path inside the container, and read-only state.
2. **Given** a mount, **When** they open it, **Then** they see its folders first and then its files, each with name, size and modification time, and can walk into subfolders and back up to the mount's root, never above it.
3. **Given** a text file up to the editor limit, **When** they open it, **Then** it opens in the editor with line numbers, highlighting chosen from its name or type, search, and soft wrap.
4. **Given** a binary file, or a text file above the editor limit, **When** they open it, **Then** Depths shows its size and type and offers a download, but does not load it into the editor.
5. **Given** a person who may only view the stack, **When** they open the stack, **Then** there is no Files tab and the server refuses any file request they send.
6. **Given** a stack whose containers have never been created, **When** the Files tab opens, **Then** it lists the mounts the compose file declares and explains that named volumes appear once the stack has been deployed.

---

### User Story 2 - Edit and save a configuration file (Priority: P1)

The person changes `adminlist.txt`, or the server's `start_server.sh` arguments, and saves. Depths
writes the file in place and keeps its owner and permissions, so the game server can still read
it. Because a game server can overwrite its configuration when it shuts down, Depths tells them the
service is running and offers to save and then restart that service.

**Why this priority**: This is the request. Changing a config file without a shell is the reason
the feature exists.

**Independent Test**: Open a text file in a writable mount, change one line, and save. The file on
disk has the new content, the same owner, group and mode, and the save appears in the audit log
with the stack, service, mount and path. Saving the same file again after it changed on disk is
refused with a choice to reload or compare.

**Acceptance Scenarios**:

1. **Given** an open text file in a writable mount, **When** the person saves, **Then** the file on disk holds exactly the editor's content, with the owner, group and permission bits it had before.
2. **Given** a file that changed on disk after it was opened, **When** they save, **Then** the save is refused, and they can reload the file (losing their edit), or compare their version with the one on disk and then save over it.
3. **Given** the service that mounts the file is running, **When** they save, **Then** they are told the service may overwrite or ignore the change until it restarts, and are offered **Save** or **Save and restart <service>**. The second option is offered only if they may restart the stack.
4. **Given** a file whose format Depths can check (JSON, YAML, TOML, XML, INI-style), **When** its content does not parse, **Then** the editor marks the error with its line before saving, and saving still works after a confirmation. Depths never refuses a save because of format.
5. **Given** a mount that is read-only in every container that uses it, **When** they open a file in it, **Then** the editor is read-only and says why.
6. **Given** unsaved changes, **When** they leave the file or the page, **Then** they are asked to confirm before losing them.

---

### User Story 3 - Download and upload files, for backups and restores (Priority: P2)

Before a risky change, the person downloads the `worlds_local` folder as a single archive. Later
they upload a world file to put an older save back, after stopping the server.

**Why this priority**: World saves are the data people care about most, and moving them in and out
is the second most common reason for a shell today. It builds on browsing but not on editing.

**Independent Test**: Download a folder holding several files and a subfolder, then compare the
archive's contents with what is on disk. Upload a file over an existing one: the new content is on
disk with the old owner and mode, and the replaced version can be restored (US5).

**Acceptance Scenarios**:

1. **Given** a file, **When** the person downloads it, **Then** they receive it byte for byte.
2. **Given** a folder, **When** they download it, **Then** they receive one archive of the folder with its structure, up to the download limit. Above the limit Depths says how large the folder is and does not start the download.
3. **Given** a writable folder, **When** they upload one or more files up to the upload limit, **Then** each lands in that folder. A name that already exists is replaced only after they confirm.
4. **Given** the service using the mount is running, **When** they upload over an existing file, **Then** they are warned that the server may hold the file open or overwrite it, and are offered to stop the service first.
5. **Given** an upload or download, **When** it completes or fails, **Then** it is recorded in the audit log with its size.

---

### User Story 4 - Create, rename and delete files and folders (Priority: P3)

The person creates a missing `permittedlist.txt`, renames an old world to keep it aside, and
deletes a crash dump.

**Why this priority**: Useful housekeeping, but most config changes only edit existing files.

**Independent Test**: In a writable mount, create a file and a folder, rename the file, then delete
both. Each step is reflected on disk and in the audit log. New entries take the owner and group of
the folder they are created in.

**Acceptance Scenarios**:

1. **Given** a writable folder, **When** they create a file or folder, **Then** it is created with the owner and group of the folder that holds it, and the usual default permissions.
2. **Given** a file or folder, **When** they rename or move it within the same mount, **Then** it moves. Moving between mounts is not offered.
3. **Given** a file or folder, **When** they delete it, **Then** Depths asks for confirmation that names it and, for a folder, says how many entries it holds. A deleted file can be restored from its history (US5) while that history is kept.

---

### User Story 5 - Undo a bad change from the file's history (Priority: P3)

A save breaks the server's start. The person opens the file's history, sees the versions Depths
kept before each of its own writes, compares one with the current file, and restores it.

**Why this priority**: A safety net that makes people confident to edit, but the feature works
without it.

**Independent Test**: Save a file three times, then open its history. Three earlier versions are
listed with time and author. Restoring the first one puts its exact content back and is itself
recorded as a new save.

**Acceptance Scenarios**:

1. **Given** Depths is about to overwrite, replace or delete a file, **When** the file is within the history size limit, **Then** Depths keeps its previous content outside the volume, with time and author.
2. **Given** a file with history, **When** they open its history, **Then** they see the kept versions, newest first, can compare any version with the current file, and can restore it.
3. **Given** the history limits (versions per file, total size, age), **When** a limit is reached, **Then** the oldest versions are dropped first.

---

### Edge Cases

- **Symbolic links**: a link whose target stays inside the mount is shown and followed. A link that
  points outside the mount (absolute, or `..` beyond its root) is shown as a link with its target,
  and is never followed, opened, written through or downloaded through.
- **Hard links**: a file with more than one hard link is read-only in the explorer. A write would
  also change the other name, which could be outside the volume.
- **Special files**: device nodes, sockets and FIFOs are listed as such and never opened, read or
  written.
- **Sensitive bind mounts**: a bind mount that the 051 rules rate root or escape (the host's `/`,
  `/etc`, `/proc`, the runtime socket, and so on) is listed but not browsable, with the reason. The
  explorer never becomes a way into the host's own files.
- **Mounts shared by several services**: the mount is listed under each service, and is writable
  when at least one service mounts it writable.
- **Interpolated or relative sources**: bind sources are resolved the way compose resolves them,
  relative to the stack directory. A source that cannot be resolved is listed as unresolved, not
  guessed.
- **Very large folders**: a folder with tens of thousands of entries lists quickly and scrolls
  smoothly. Filtering by name works within the folder.
- **Permission denied on disk**: a file Depths cannot read or write shows the error the system
  gave, verbatim, and nothing is changed.
- **Disk full or a write that fails part way**: a save is all or nothing. The file holds either its
  old content or the new one, never a mix.
- **Encodings and line endings**: a file's line endings (LF or CRLF), its trailing newline and a
  leading byte-order mark survive an edit the person did not make to them. A file that is not valid
  UTF-8 is treated as binary.
- **Concurrent editors**: two people editing the same file: the second save is refused as changed on
  disk (US2 scenario 2).
- **Container removed while browsing**: the explorer keeps working for named volumes and bind mounts.
  It reports when a mount no longer exists.
- **Foreign projects and standalone containers**: not in scope. Their volumes are not shown.

## Requirements *(mandatory)*

### Functional Requirements

**Mounts and browsing**

- **FR-001**: Depths MUST list, per service of a managed stack, every mount its containers use, or
  that its compose file declares when no container exists: named volumes and bind mounts, with
  source, path inside the container and read-only state. Temporary in-memory mounts are listed as
  not browsable.
- **FR-002**: Users MUST be able to browse a mount's folders, with entries sorted folders first,
  then by name. Each entry shows its name, kind (file, folder, link, special), size, modification
  time, and owner and permissions in a compact form.
- **FR-003**: Every path a request names MUST be resolved inside the chosen mount's root, after
  resolving `..` and links. A request that would resolve outside the root MUST be refused, audited,
  and leave nothing changed. This check MUST be made on the server for every operation, at the
  time of the operation.
- **FR-004**: Bind mounts whose host path the 051 rules rate root or escape MUST NOT be browsable.
  Depths shows them with the reason. Links pointing outside the mount, hard-linked files (for
  writing) and special files follow the Edge Cases rules.
- **FR-005**: Listing a folder MUST stay responsive with at least 50,000 entries, and MUST let the
  user filter the listing by name.

**Viewing and editing**

- **FR-006**: Text files up to an editor limit (default 2 MiB, configurable) MUST open in a code
  editor with line numbers, syntax highlighting chosen from the file name or content (at least
  INI/CFG, JSON, YAML, TOML, XML, properties, shell scripts, Lua and plain text), search and
  replace, undo and redo, soft wrap, and keyboard use throughout. Binary or larger files offer
  download only.
- **FR-007**: Saving MUST replace the file's content all-or-nothing, and keep its owner, group and
  permission bits. Line endings, trailing newline and byte-order mark MUST be preserved unless the
  user changed them.
- **FR-008**: A save MUST be refused when the file changed on disk since it was opened. The user can
  reload it, or compare their version with the disk's and save over it deliberately.
- **FR-009**: For formats Depths can check (JSON, YAML, TOML, XML, INI-style), parse errors MUST be
  shown in the editor with their line. They warn and never block a save.
- **FR-010**: When a running container mounts the file's mount, the save MUST say so and offer
  **Save and restart <service>** to people who may restart the stack. The restart is the ordinary
  lifecycle action, with its own permission check and audit entry.
- **FR-011**: Files in mounts that are read-only in every container that uses them MUST open
  read-only, and the server MUST refuse writes to them.

**Transfer and file management**

- **FR-012**: Users MUST be able to download a file, or a folder as one archive, up to a download
  limit (default 2 GiB, configurable). Downloads MUST be streamed, never held whole in memory.
- **FR-013**: Users MUST be able to upload files into a writable folder, up to an upload limit per
  file (default 1 GiB, configurable). An upload is all-or-nothing per file, and replacing an
  existing name requires confirmation.
- **FR-014**: Users MUST be able to create files and folders, rename or move entries within one
  mount, and delete files and folders after a confirmation that names what will be deleted. New
  entries take the owner and group of their parent folder.

**History**

- **FR-015**: Before Depths overwrites, replaces or deletes a file of up to the history size limit
  (default 10 MiB per file), it MUST keep the previous content in Depths' own data, not in the
  volume, with time, author, stack, mount and path. Limits on versions per file (default 20),
  total history size (default 1 GiB) and age (default 30 days) are configurable, and the oldest
  versions go first.
- **FR-016**: Users MUST be able to list a file's kept versions, compare any with the current file,
  and restore one. A restore is itself a save, with history.

**Access, trust and audit**

- **FR-017**: Browsing, opening, downloading and every change MUST require the edit permission on
  that stack. Viewers get no Files tab, and the server refuses their file requests. This is checked
  on the server for every request, the same way as other stack actions.
- **FR-018**: Under the 052 restricted view (an untrusted network), a person MAY list folders but
  MUST NOT open, download, upload or change files. Configuration files often hold passwords that
  are not masked the way `.env` values are.
- **FR-019**: Every open of a file's content, download, upload, save, create, rename, move, delete
  and restore MUST be recorded in the audit log, with who, when, stack, service, mount, path and
  size. Every refusal, outside-the-root attempts included, MUST be recorded too. File contents MUST
  NOT be written to the audit log.
- **FR-020**: Depths MUST reach a volume's files with no more access than that one mount. It MUST
  NOT gain standing read or write access to the host's filesystem, the Docker data directory or
  other stacks' volumes to provide this feature, and its own lockdown (read-only root, dropped
  capabilities) MUST stay as it is.
- **FR-021**: File operations MUST act with the permissions of the files' owner, not as an
  all-powerful user. A file its owner could not write stays unwritten, and the error says so.
- **FR-022**: The feature MUST be switchable at runtime by the operator: off, read-only (browse,
  open and download only) or full. The default is full. The UI MUST mirror the setting, and the
  server MUST enforce it.

**Interface**

- **FR-023**: The explorer MUST follow Depths' layout rules. On a phone it is one pane (the listing,
  then the open file full width). On wider screens the tree or listing and the editor sit side by
  side. It works by touch, mouse and keyboard alone, with no sideways page scroll.
- **FR-024**: The URL MUST name the stack, mount and path, so a folder or file can be bookmarked,
  shared with someone who has access, and reopened after a reload.

### Key Entities

- **Mount**: a place a service's container sees files: its kind (named volume, bind, temporary),
  source (volume name or host path), path inside the container, read-only state per service, and
  whether it is browsable (with the reason when not).
- **Entry**: one item in a mount's folder: name, kind (file, folder, link with its target, special),
  size, modification time, owner, group, permission bits, link count, and a content fingerprint for
  change detection when it is a file.
- **File version**: a copy of a file's content kept by Depths before one of its writes: stack,
  mount, path, time, author, size and the content. Bounded by the history limits.
- **File audit event**: who did what to which path in which mount of which stack, when, with the
  size, the outcome and the refusal reason if there was one.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From a stack's page, a person opens a configuration file in a service's volume in at
  most 4 clicks or taps, and saves an edit to it in under 1 minute, without a shell.
- **SC-002**: A folder of 50,000 entries lists in under 2 seconds on the reference host, and scrolling
  and filtering it stays smooth (no input waits longer than 100 ms).
- **SC-003**: Over a test suite of at least 30 escape attempts (`..`, absolute paths, links out of
  the mount, links swapped between check and use, hard links, special files, sensitive bind mounts),
  zero reads or writes reach anything outside the chosen mount, and every attempt is audited.
- **SC-004**: In 100% of saves, uploads and restores in the test suite, the file keeps its owner,
  group and mode, and is never left half-written when the write fails part way.
- **SC-005**: A person who changed a file by mistake can put the previous version back in under
  30 seconds from the file's history.
- **SC-006**: Downloading a 1 GiB folder does not raise Depths' memory use by more than 64 MiB.

## Assumptions

- The audience is the stack's editors: people who run game and home servers and today change these
  files over SSH. Viewers do not need to read volume contents, and many configs hold secrets.
- "Volume" covers named volumes and bind mounts. Foreign projects, standalone containers and
  volumes no managed stack uses are out of scope for this version.
- Depths keeps its current lockdown. Reaching a volume will go through a short-lived, single-purpose
  helper that sees only the chosen mount, created through the same restricted runtime access Depths
  already has. The plan settles the mechanism, and the operator does not configure it.
- Editing through the explorer gives no more power than editing the compose file already does. Both
  are gated by the same edit permission, and the network trust gate (052) restricts both.
- The existing compose editor's component is reused and extended for highlighting of more formats,
  so the two editors behave the same.
- The default limits (2 MiB editor, 2 GiB download, 1 GiB upload, 10 MiB history per file, 20
  versions, 1 GiB total, 30 days) suit game and home servers and are configurable.
- The history lives in Depths' data directory, follows the 050 storage approach, and is covered by
  the 050 backup guidance.
- Game-specific knowledge (for example, which Valheim file holds the admin list) is not built in.
  The explorer is generic. Highlighting comes from file names and content.
