# People and access

Depths keeps its own list of who may do what. Signing in proves who someone is; the list decides
what they see.

| Role | What they can do |
|---|---|
| **Pending** | Nothing. They see "You don't currently have access" and a sign-out button, and nothing about the host |
| **User** | Only the stacks assigned to them: view (with secret-looking `.env` values masked), start, stop, restart, update, edit the compose and `.env` files (a masked value they leave as it is keeps its real value), follow logs, watch stats and, when shells are on, open shells |
| **Admin** | Everything: every stack, revealing masked `.env` values (each reveal is recorded in the audit log), creating and deleting stacks, `down` with volumes or images, read-only views of projects that are not in the stacks directory, the audit log, and managing people |
| **Owner** | An admin named in the server's settings. Always admin, and cannot be demoted or removed from the UI |

A user with no stacks assigned has no access, the same as a pending person.

## The owner

The first admin has to come from somewhere, so Depths reads it from the environment:

```sh
OAUTH_PROVIDER=discord
OWNER_USER_ID=<your Discord user id>
```

`OWNER_USER_ID` (or `DEPTHS_OWNER_USER_ID`, or `--owner-user-id`) takes one id or several,
comma-separated. Each is written as `<provider>:<id>`:

| Form | Example |
|---|---|
| `discord:<user id>` | `discord:123456789012345678` |
| `github:<login>` or `github:<numeric id>` | `github:octocat` or `github:583231` |
| `oidc:<sub>` | `oidc:3f1c2a9e-...` |

A bare value with no `provider:` prefix belongs to `OAUTH_PROVIDER`. With no `OAUTH_PROVIDER`
set, a bare value stops start-up. An owner whose provider is not configured also stops start-up.

```sh
# several owners, provider-qualified
OWNER_USER_ID=discord:123456789012345678,github:octocat
```

An owner given as a GitHub login is pinned to that account's numeric id the first time it signs in,
and matched by id from then on. If the login is later renamed and someone else registers it, they
do not become an owner.

If no owner is configured and the users list holds no admin, Depths refuses to start and names
`OWNER_USER_ID`, so you cannot end up with a Depths no one can manage.

## Finding your id

**Discord.** In Discord, open **User Settings > Advanced** and turn on **Developer Mode**. Then
right-click your name (or tap and hold it) and choose **Copy User ID**. It is a long number.

**GitHub.** Your login works for `OWNER_USER_ID`. For the numeric id, which the "Add a person by
id" form needs, open `https://api.github.com/users/<login>` and use the `id` field.

**OIDC.** The id is the `sub` claim of the person's token. Your identity provider's admin console
shows it on the user's page (Keycloak calls it the user's ID). It is not the username.

You do not always need to look ids up: anyone who signs in appears in the users list, with their
id, under **Waiting for access**.

## Letting people in

Admins manage people on the **Users** page (`/admin/users`, linked in the navigation for admins only). The page has three parts.

**Waiting for access.** Everyone who has signed in but has no access yet, with their name, their
provider id and when they were last seen. To let someone in:

1. Ask them to sign in once. They will see "You don't currently have access"; that is expected.
2. Find them under **Waiting for access** and choose **Admit**.
3. Pick a role. For **User**, tick the stacks they should have under **Stacks**.
4. **Save**. Their access applies from their next action; they reload the page to see their
   stacks.

**People with access.** Everyone with the user or admin role. Each row shows their stacks (or
"Every stack." for an admin), and **Edit** and **Remove**. A stack name marked **(missing)** is
assigned but no longer on disk; it comes back if the directory does.

**Add a person by id.** Pre-add someone who has never signed in: type `discord:<id>`,
`github:<numeric id>` or `oidc:<sub>`, pick a role, and **Add**. They get that access the first time
they sign in with that account. To give them stacks, **Edit** their row afterwards.

## Changing and removing access

- Changes apply from the person's next action: the list is read on every command, not stored in
  their session.
- **Removing** someone, or taking away their last stack, signs them out and closes their open
  tabs straight away.
- If a removed person signs in again, they come back as pending.
- Owners show an **Owner** badge and cannot be edited here. To change the owner, change
  `OWNER_USER_ID` and restart Depths.

Every change (`user_added`, `user_changed`, `user_removed`) is recorded in the audit log, on the
**Audit** page (`/admin/audit`), which admins can also export.

## Sessions and networks

Everyone can see their own sessions and networks on **Your sessions** (the person icon in the top
bar): each signed-in browser with its address, approximate location and whether its network is
trusted, and each network with your *home* or *public* answer. You can end any other session, or
all of them at once, from any network. See [Networks and the restricted view](security.md).

Admins see the same for anyone: on the Users page, open **Sessions and networks** on their row.

- **End every session** signs that person out everywhere. It works from any network.
- **Whitelist** a network (an address or a CIDR such as `198.51.100.0/24`, with an optional
  label) to trust it for that person at once, without the waiting period.
- **Remove whitelist** takes the entry away; the network is then unknown again.
  **Forget** removes a network Depths has only seen, so it counts as new next time.

Whitelisting, removing and forgetting are only possible from a trusted network. From anywhere
else the buttons are disabled and the server refuses the request. Every change is in the audit
log (`network_whitelist_add`, `network_whitelist_remove`, `network_forget`, `session_end`).

## Where the list lives

The list is `users.json` in the data directory (`/var/lib/depths` in the example compose file),
mode 0600. Depths is its only writer: it writes a new file and renames it over the old one, so a
crash never leaves half a file. Do not edit it by hand. If it ever fails to parse, Depths refuses
to start and names the file and the problem, rather than starting with an empty list.

Back up the data volume. Without `users.json`, everyone except the owners is pending again.
