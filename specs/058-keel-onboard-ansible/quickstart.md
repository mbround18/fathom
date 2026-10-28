# Quickstart: validating 058

Each scenario below maps to a user story. Commands run from `../ansible` unless noted.

## Prerequisites

- Keel reachable over HTTPS (dev compose stack or the server host), signed in through Keycloak.
- A throwaway Debian VM listed in `inventory/hosts.yml` under its hostname, with ignored
  host_vars per [trust-vars](contracts/trust-vars.md) § Trial.
- A runner build that includes amendments A1 and A2 ([keel-amendments](contracts/keel-amendments.md)).

## 1. Root paths are closed (US1)

```sh
make test ROLE=base
make test ROLE=keel_runner
```

Expect: every case in [base-hardening](contracts/base-hardening.md) § Tests and in A1 § Tests
passes. On the VM, as an `automation` member:

- `touch /root/ansible/.git/hooks/x` fails;
- `sudo automation-compose-up ../x` exits 64;
- `sudo -l` lists no `docker` rule.

## 2. One command (US2)

On the VM, as root, in `/root/ansible`:

```sh
KEEL_TOKEN=<enroll token> make keel-node
```

Expect:

- `debian` then `keel-node` run;
- the host appears enrolled in Keel within a minute;
- `/etc/sudoers.d/keel-exec` exists, and no token remains on disk.

Repeat on a second VM with a join token. Expect a fingerprint in the output, and the host
waiting in Keel until you accept it.

Then, on a VM that ran the old trial role, run `make keel-node` again. Expect:

- the migration;
- the `keel-exec` user owns `/var/lib/keel-executor`;
- the `automation` group is still present;
- the ledger is unchanged.

## 3. Jobs and triggers (US3)

```sh
make lint   # includes scripts/keel-manifest-check.py
```

- Commit a change under `roles/base/` and push. Keel queues `debian` (and every profile that
  reaches `base`), and not `keel-node`.
- Commit an edit to `inventory/hosts.yml`. Nothing is queued.
- `upgrade` appears only as a manual run.

## 4. Trust (US4)

- On the VM, remove its ignored host_vars and re-run `make keel-node`. Expect
  `require_signed: true`; an unsigned commit is refused.
- In Keel, approve an override with a Keycloak session that has no pinned passkey. Expect
  refusal.

## 5. Outside runs and break-glass (US5, US6)

On the VM: `cd /root/ansible && make debian`. Within 60 s, Keel shows **Changed outside Keel**
for the host, with the commit and whether the tree was dirty.

- Approve a re-baseline. The badge clears.
- Run `CHECK=1 make debian`. No badge.
- Stop the Keel server, then run `make debian` on the VM. It still works. After the server
  restarts, the badge appears.

## 6. Order gate (US5)

Before each host in research R12's order, record four things:

- it is enrolled;
- its baseline is approved;
- its first runs are clean;
- a `make` run on it shows as outside Keel.

Don't start the next host until all four hold.
