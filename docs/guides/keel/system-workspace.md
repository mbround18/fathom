# The System workspace

Keel can run Ansible against the host it runs on, so the playbooks that keep Keel's own server
patched and hardened can live in a repository and go through review like any other change. Doing
this needs a special workspace, the **System workspace**. Only the instance **owner** can see it
and use it.

It is off until the owner sets it up.

## Who can use it

| Role | What they see |
|---|---|
| Owner | The **System** tab in Settings, the System workspace, its hosts, runs and audit entries |
| Admin | Nothing. The workspace and its rows do not appear in any list, and opening it directly is refused |
| User | Nothing |

The owner must be signed in to use it. API tokens can't be created for the System workspace,
and none of them work on it. Nobody can be granted access to it either: it belongs to the owner
role, not to a team.

## Setting it up

1. Open **Settings → System** and choose **Set up**.
2. Pick the **allowed profiles**: the Ansible profiles the Keel host may be enrolled with (1 to 16).
   An enrolment that asks for any other profile is refused with `profile_not_allowed`.
3. Confirm. You'll be asked to re-authenticate. Keel creates the workspace `keel-system` and
   records `system_configure` in the audit log.

Then do what you would for any workspace:

- **Bind a repository** from the System tab. It uses the same form as other workspaces.
- **Enrol the Keel host.** Open Enroll, pick the System workspace and run the snippet on the
  Keel server. See [the enrolment snippet](enrollment-template.md).
- **Run jobs** from the workspace as usual. Runs are reviewed and confirmed like any others.

You can change the allowed profiles later from the same tab. If someone else changed them first,
you'll get `system_conflict`. Reload and try again.

The name `keel-system` is reserved. No other workspace can be created with that slug.

## Removing it

**Remove** on the System tab asks you to type `keel-system`, then in one step:

- revokes the hosts enrolled in it, along with their certificates;
- burns any enrolment tokens that haven't been used;
- disables its repository bindings;
- revokes API tokens scoped to it;
- archives the workspace.

The audit log records `system_remove` with the counts. Running jobs are stopped. To use the
feature again, set it up again. The archived workspace is restored, and you'll need to enrol
the host again.

The System workspace can't be archived from the workspace list. Use **Remove**.

## When Keel restarts during a run

A run on the Keel host can restart Keel itself, for example when a playbook upgrades it. The run
first shows as `failed` with the reason `runner_lost`, because Keel lost track of the host
while it was down.

The host's runner keeps the result until Keel confirms it has it. When the runner reconnects,
it sends the result again, and Keel corrects the run to its real outcome. The audit log records
this as `run_result_late`. A result Keel already has is ignored, so sending it twice does no
harm.

The runner keeps unconfirmed results for **30 days** by default. To change this, set
`--max-offline` or `KEEL_RUNNER_MAX_OFFLINE` (for example `7d`, `12h` or `90m`) on the host. An
older result is dropped without being sent, and the runner logs one warning line naming the run.
The results are kept in `undelivered/` under the runner's state directory, readable only by the
runner.

This applies to every host, not just the Keel host. A runner that loses its connection at the
end of a run delivers the result when it reconnects.
