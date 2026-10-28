# Contract: base-profile hardening (US1, FR-001, FR-003)

What `roles/base` guarantees on every host after this change, whether or not Keel is installed.
Decisions in research R1 and R2.

## `/root/ansible`

| Path | Owner | Mode | `automation` group ACL (access and default) |
|---|---|---|---|
| `/root/ansible` and everything below, `.git` included | root:root | unchanged | `r-X` |

- The role removes any `w` for the group, recursively, in both the access and the default ACL. It
  is idempotent: a second run reports no change.
- If `/root/ansible` does not exist, nothing is created (as today).

## `/etc/sudoers.d/automation`

Validated with `visudo -cf`, mode `0440`. Exactly:

```text
Cmnd_Alias AUTOMATION_GIT = /usr/bin/git -C /root/ansible pull --ff-only, \
                            /usr/bin/git -C /root/ansible status --short, \
                            /usr/bin/git -C /root/ansible fetch --all
Cmnd_Alias AUTOMATION_COMPOSE = /usr/local/sbin/automation-compose-up
%automation ALL=(root) NOPASSWD: AUTOMATION_GIT, AUTOMATION_COMPOSE
```

`base_automation_sudo_commands` is removed as a free-form list. US6 cleanup sets
`base_automation_git_rules: false`, which drops `AUTOMATION_GIT`.

## `/usr/local/sbin/automation-compose-up`

Root-owned `0755`, POSIX `sh`, with no sourcing and no `eval`.

| Input | Result |
|---|---|
| no argument, or more than one | exit 64, usage on stderr |
| argument not matching `^[a-z0-9][a-z0-9-]{0,62}$` | exit 64 |
| name not a whole line of `/etc/automation/composes.list` | exit 77, "not on the approved list" |
| `/root/ansible/composes/NAME/compose.yaml` missing | exit 66 |
| otherwise | `exec env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin docker compose -f /root/ansible/composes/NAME/compose.yaml up -d` |

## `/etc/automation/composes.list`

- root:root `0644`. One name per line, from `base_automation_composes`.
- Default: `[inventory_hostname]` if `composes/<inventory_hostname>/compose.yaml` exists in the
  checkout, otherwise empty.
- A name that is renamed or removed from `composes/` stays listed until the variable changes, and
  the wrapper then fails with exit 66, not by starting something else.

## Tests (Molecule, `roles/base`)

- A group member can write neither `.git/hooks/x` nor `.git/config` nor `playbooks/x.yaml`.
- `sudo automation-compose-up 'x up -d -f /tmp/y'`, `../x`, an unlisted name, and two arguments
  are all refused with the exit codes above.
- A listed name starts its project.
- `sudo -l -U <member>` shows no `docker` rule.
- A second converge reports no changes.
