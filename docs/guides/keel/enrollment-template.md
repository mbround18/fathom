# The enrolment snippet

When someone enrols a host, Keel shows the one-time token once, together with a ready-to-copy
command: the enrolment snippet. Instance owners and admins decide what that command looks like, and
can add instructions that appear under it.

Both live in **Settings → Enrollment**. Saving asks you to confirm with your passkey, and the
change is recorded in the audit log with the text before and after.

## Placeholders

The snippet is a template. Keel replaces each placeholder when it issues a token.

| Placeholder | Becomes | Example |
|---|---|---|
| `%t` | The one-time enrolment token | `keel-enroll-v1.…` |
| `%n` | The host's inventory name | `web-01` |
| `%p` | The host's profiles, comma-separated | `base,compose` |
| `%w` | The workspace's slug | `home` |
| `%s` | The address hosts use to reach Keel's runner listener, as `host:port` | `keel.example.net:7831` |
| `%u` | Keel's public URL | `https://keel.example.net` |
| `%e` | When the token expires, in UTC | `2026-10-28T12:15:00Z` |
| `%%` | A literal `%` | `100%` |

Rules Keel checks when you save:

- **The token must be in it.** A template without `%t` is refused, since the snippet would be
  useless.
- **Only the placeholders above.** Anything else after a `%`, such as `%x`, `%T` or a `%` at the
  very end, is refused. Write `%%` when you mean a percent sign, for example in `printf '%%s'`.
- **At most 4 KB.** Instructions can be up to 20 KB.

Values are put in once and never read again as placeholders. A host named `%t` shows up as `%t`,
not as the token.

The editor shows a live preview with a made-up token, so you can see the result as you type.

## The default

Until someone saves a template, Keel uses one that matches the Keel Ansible repository, which reads
the token from `KEEL_TOKEN`:

```sh
read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t
make keel-node
```

The person enrolling runs the first line, pastes the token at the hidden prompt, and then runs
`make keel-node`. The token never lands in shell history. **Restore default** in the editor puts
this text back; restoring it is saved and audited like any other change.

## Examples

Pass the token as an Ansible variable, limited to the new host:

```sh
read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t
ansible-playbook site.yml --limit %n -e keel_profiles=%p
```

A one-line environment file for a script that reads it:

```sh
printf 'KEEL_URL=%u\nKEEL_RUNNER=%s\nKEEL_TOKEN=%t\n' > keel.env  # expires %e
```

Because values are put in exactly as they are, quote placeholders yourself when your shell needs it.

## Instructions

The instructions are Markdown, shown below the snippet in the Enroll dialog. Tables, task lists and
code blocks work. To keep the dialog safe:

- **Raw HTML is shown as text**, not run.
- **Links open in a new tab.**
- **Images are not loaded.** They appear as a link to the image instead.

If Keel can't read its settings when a token is issued, it falls back to the default template, and
the dialog says the instructions are unavailable. The token itself is still issued.

## Tokens after the fact

Keel stores only a hash of each enrolment token, so a token is shown once and never again. To give
someone a fresh one, open **Settings → Tokens**, find the enrolment, and choose **Roll**. Keel
revokes the old token if it is still pending and shows the new one, with the snippet, once.
