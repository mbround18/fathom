# Contract: host install through the base profile (`../ansible`, role `keel_runner`)

This is what the operator's Ansible repository must do to bring a Debian host under Keel
(research R22). The role is written in `../ansible`, not in this repository; this contract is
what Keel relies on. Nothing in `../ansible` changes as part of 055 without the owner's go-ahead.

## Variables

| Variable | Default | Notes |
|---|---|---|
| `keel_runner_enabled` | `false` | Per host or group. |
| `keel_runner_version`, `keel_runner_sha256` | required | Release and checksum per architecture, for `github_release`. |
| `keel_runner_server` | required | `host:port` of Keel's runner listener. A host name, not an IP (the repository keeps IPs out of Git). |
| `keel_runner_host_ca` | required | The `@cert-authority` key from `GetSshCa`. |
| `keel_runner_origin`, `keel_runner_rp_id` | required | Keel's web origin and passkey RP id. |
| `keel_runner_trusted_signers` | `[]` | SSH `allowed_signers` lines and OpenPGP public keys. |
| `keel_runner_approvers` | `[]` | `{ label, credential_id, public_key_cose, alg }`, from Keel's passkey page. |
| `keel_runner_require_signed` | `false` | `true` makes the host refuse unsigned work whatever Keel says. |
| `keel_runner_allowed_profiles` | `[]` | The ceiling for profiles Keel may assign. Empty means the host refuses every job. |
| `keel_runner_override_quorum` | `1` | |
| `keel_runner_inventory_name` | `inventory_hostname` | |
| `keel_enroll_token` | unset | Passed only with `-e` for the first run. Never stored in Git or in vars files. |

Validation: the role fails if `keel_runner_require_signed` is true and either signer or approver
list is empty, because such a host could never run anything.

## What the role does

1. **Accounts:**
   - `keel-runner`: system user, `/usr/sbin/nologin`, no home, not in `sudo`, `docker`,
     `automation` or any group a human belongs to; member of `keel-trust`.
   - `automation`: system user, `/usr/sbin/nologin`, home `/var/lib/keel-executor` (`0700`);
     member of `keel-trust`. Its primary group is its own (`automation` stays a supplementary
     group for humans, as today), so executor files carry no group access.
   - `/etc/sudoers.d/keel-automation`, validated with `visudo -cf`:
     `automation ALL=(root) NOPASSWD: ALL` and `Defaults:automation !requiretty`. This is what
     Ansible `become` needs. It is root-equivalent; the executor's checks are the gate.
2. **Binaries:** `/usr/local/bin/keel-runner` and `/usr/local/bin/keel-executor` from
   `github_release`, checksum-verified, root-owned `0755`.
3. **Ansible:** `/opt/keel-runner/venv` built with `uv` from a hash-pinned requirements file
   shipped in the release; root-owned and not writable by either account. The `keel_events`
   callback is installed into it.
4. **Trust files:** `/etc/keel-runner/trust/` as in contracts/signed-mode.md: directory `0750`,
   files `0640`, owner root, group `keel-trust`.
5. **Units:**
   - `keel-runner.service`: `User=keel-runner`, `ExecStart=/usr/local/bin/keel-runner`,
     `StateDirectory=keel-runner` (`0700`), and the hardening in research R2. `Restart=on-failure`
     is acceptable here: these are server hosts managed by Ansible, not compose services on the
     dev box.
   - `keel-executor.socket`: `ListenStream=/run/keel-runner/executor.sock`, `SocketUser=root`,
     `SocketGroup=keel-runner`, `SocketMode=0660`, `Accept=yes`.
   - `keel-executor@.service`: `User=automation`, `ExecStart=/usr/local/bin/keel-executor`,
     `StandardInput=socket`, `KillMode=control-group`, `PrivateTmp=yes`, `ProtectHome=yes`,
     `RuntimeMaxSec=` from the job deadline cap (default `6h`). `NoNewPrivileges` stays off,
     because `become` needs sudo.
6. **Host key:** generated on first start by `keel-runner` (`ssh-keygen`-compatible Ed25519) in
   its state directory. It never leaves the host.
7. **Enrollment:** if `keel_enroll_token` is set and the host is not yet enrolled, write it to
   `/var/lib/keel-runner/enroll-token` (`0400`) with `no_log: true`, then start the service.
8. **Idempotence:** re-running the role changes trust files and versions only. It never deletes
   `/var/lib/keel-executor/ledger.jsonl` or the executor's Git store.

## Changing trust

Approvers, signers, `require_signed` and `allowed_profiles` change by committing to the
repository's vars. Once a host is under Keel, that commit reaches it through Keel like any other,
so in signed mode it must be signed or approved under the **old** trust (research R22). Keel's
review flags any diff touching `keel_runner_*` variables.

## Prerequisites in `../ansible` (research R22)

Before `automation` becomes a user that runs jobs, the base role's existing automation group
grants must stop being root-equivalent:

1. The recursive group ACL on `/root/ansible` must not give write on `.git`, or root must stop
   running `git` there.
2. The `docker compose -f /root/ansible/composes/* up -d` sudo rule must become a fixed-list
   wrapper.

## Uninstall

`keel_runner_enabled: false` stops and disables the units, removes the binaries, the venv, the
sudo rule and the trust files, and keeps `/var/lib/keel-executor` (the ledger is history).
Revoking the host in Keel is a separate step.
