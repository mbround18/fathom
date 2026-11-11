# Hosts and enrolment

Keel only runs on hosts that have enrolled. A host enrols with a one-time token, connects back to
Keel, and from then on reports to its workspace. This guide covers enrolling a host, checking that
the host and Keel trust the same certificate authority, and running the base profile that enrols
it.

Instance owners and admins decide what the enrolment command looks like: see
[the enrolment snippet](enrollment-template.md).

## Enroll a host

Enrolling a host needs the Operator role or higher in the workspace.

1. Open **Infrastructure → Hosts** and choose **Enroll host**.
2. Check the **Workspace**. It starts on the current one.
3. Enter the **Inventory name**: the host's name in the inventory, as Ansible sees it. Use letters,
   digits, dots, underscores and dashes, starting with a letter or digit.
4. Under **Profiles**, list the profiles the host serves. `base` is there to start with. Press
   Enter or type a comma to add another.
5. Choose **Continue**. Keel shows what it will do. Choose **Confirm**. If the dialog says
   **Needs your passkey.**, confirm with your passkey.
6. The **Host token** dialog shows the **Token** (hidden until you choose to reveal it) and the
   **Snippet**. Copy what you need, then run the snippet on the host. See
   [Run the base profile](#run-the-base-profile).
7. Choose **Done**.

The enrolment token covers one host and the profiles you chose for it, and nothing else. It is
shown once, works once, and lasts 15 minutes. The dialog counts down the time left. Keel stores only
a hash of it. If you close the dialog early, Keel warns you first: choose **Keep open** to go back,
or **Close anyway**.

The inventory name matters. A run reaches the hosts listed in its inventory, matched by name, so the
**Inventory name** must be the name your inventory uses for this host.

### When the token is refused

The host checks the token, and so does Keel. If any check fails, Keel closes the connection, burns
the token and records why in the audit log. Common reasons:

| Reason | What it means |
|---|---|
| `expired` | More than 15 minutes passed. Roll a new token. |
| `token_reused` | The token was already used. Each token works once. |
| `ca_mismatch` | The token names a host CA the host or Keel does not trust. See [Check the fingerprint](#check-the-fingerprint). |
| `inventory_name_mismatch` | The host enrolled under a different inventory name than the token names. |
| `profile_not_allowed` | The token asks for a profile outside the host's allowed profiles. |
| `host_exists` | A host with that inventory name is already enrolled in the workspace. Keel also refuses this when you mint the token. |

To get a fresh token, open **Settings → Tokens**, find the enrolment and choose **Roll**.
Rolling asks for your passkey. **Revoke** cancels a token that hasn't been used yet.

### Hosts in the list

The Hosts table shows each host's **State** (online, offline or revoked), its **Profiles**, the
versions of its **Runner** and **Ansible**, and when it was **Last seen**.

**Revoke** removes a host from Keel. It revokes the host's certificates, and runs in flight on the
host end as lost. It needs the Workspace admin role and your passkey.

A host marked **Changed outside Keel** reported an `ansible-playbook` run that Keel did not make.
Open the host to see those runs under **Runs outside Keel**. They are the host's own claims, which
Keel cannot verify. Runs with `--check` don't count. The mark clears when a review baseline for the
host is approved, or when every job assigned to the host has since run and succeeded. Tick **Only
hosts changed outside Keel** to list just those hosts.

## Add a host

**Add host** does two things in one step: it adds the host to the primary repository's inventory,
and it issues the host's enrolment token. Use it for a host the repository doesn't list yet. For a
host it already lists, use [Enroll host](#enroll-a-host). Adding a host needs the Operator role or
higher, and a [primary repository](workspaces.md#set-the-primary-repository).

1. Open **Infrastructure → Hosts** and choose **Add host**. Or, from a group on
   **Repository → Inventory**, choose **Add host here** to start with that group chosen.
2. Enter the **Inventory name**, as it will appear in the inventory.
3. Under **Groups**, choose the groups to add it to. With none chosen, it goes under `all`.
4. Under **Profiles**, list the profiles the host serves.
5. Choose **Review**. Keel shows what it will change, and any warnings, such as a host that will
   need an age key before it can decrypt the secrets its jobs read. Choose **Confirm**.
6. The **Host added** dialog shows the **Enrollment command**. Run it on the host, as for
   [Enroll a host](#enroll-a-host).
7. Under **Inventory change**:
   - with a [forge connection](workspaces.md#connect-a-forge), Keel says **Pull request opened**
     and links to it. Review and merge it.
   - without one, Keel shows a **Patch to apply by hand**. Apply it to the inventory and push.

Keel edits only the lines it adds, so the inventory's comments and layout stay as they were. An
inventory it can't edit safely, such as one written in flow style, always gets a patch.

Until the change is merged and synced, the host shows **Pending inventory** on Hosts: Keel has
enrolled it, but no run reaches it, because runs reach the hosts their inventory lists. The badge
goes once the primary repository's inventory lists the host.

## Age keys

Sops-encrypted files can only be read on a host whose age key is one of the file's recipients. When
a host's runner connects, it reports the host's age public key from `/etc/sops/age.pub`. It never
reads the private key. The host's details show it under **Age key**.

When no `.sops.yaml` rule names a host's key, a Workspace admin can choose **Propose sops
recipient**. Keel adds the key to the rule's recipients and opens a pull request, or shows a patch
to apply by hand. After it is merged, the files the rule covers still need re-encrypting for the
new key: run `sops updatekeys` for each, from a machine that can already decrypt them. See
[When a file is out of date](repository.md#when-a-file-is-out-of-date).

## Check the fingerprint

Keel has no host-key fingerprint to compare: the host's own key never leaves it, and Keel signs it
with a certificate once the token checks out. What you can check is that the host and Keel trust
the same SSH certificate authority (CA).

Every enrolment token ends with the SHA-256 fingerprint of Keel's active host CA. The runner on the
host only accepts a token that names a CA it already pins as `keel_runner_host_ca`. If they don't
match, it refuses the token.

To check them:

1. Open **Infrastructure → Hosts** and expand **SSH certificate authority**.
2. Compare the host CA's `SHA256:…` fingerprint with the last part of the token, after the second
   dot.
3. Make sure the `keel_runner_host_ca` your base profile installs is the line shown in the
   **keel_runner_host_ca** box. Copy it from there into your Ansible variables.

The same section lists Keel's runner CA. Keel also shows fingerprints for trusted signers (see
[Reviews and trusted signers](reviews.md#trusted-signers)) and credentials (see
[Bindings and credentials](bindings.md#credentials)).

## Run the base profile

The base profile is the profile from your Ansible repository that every host gets first. Run with
the enrolment token, it installs the Keel runner and enrols the host. With the default snippet, you
run it like this on the host, from a checkout of the Keel Ansible repository:

```sh
read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste the token
make keel-node
```

The `keel_runner` role then:

1. installs `keel-runner` and `keel-executor`;
2. writes the trust files under `/etc/keel-runner/trust/`;
3. saves the token where only the runner can read it, and starts the runner.

On its first start the runner makes its own Ed25519 key. It connects to Keel, enrols with the
token, saves its certificate and deletes the token. The host then shows as **online**.

The role's main variables:

| Variable | What it sets |
|---|---|
| `keel_runner_server` | Keel's runner listener, as `host:port` (a host name, not an IP) |
| `keel_runner_host_ca` | The host CA line the runner pins (see [Check the fingerprint](#check-the-fingerprint)) |
| `keel_runner_inventory_name` | The host's inventory name, `inventory_hostname` by default |
| `keel_runner_allowed_profiles` | The profiles this host will run. If empty, the host refuses every job |
| `keel_runner_trusted_signers` | The keys whose signed commits the host accepts |
| `keel_runner_approvers` | The approvers' passkeys, copied from Keel's **Passkeys** page, whose approvals the host accepts |
| `keel_runner_require_signed` | Whether the host runs only signed commits |
| `keel_runner_override_quorum` | How many approvals an override of unsigned commits needs on this host |

Running the base profile again later changes only the trust files and versions. It does not enrol
the host a second time.
