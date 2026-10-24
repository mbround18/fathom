# Depths user guides

Depths manages the Docker Compose stacks on one host from a browser. It reads a directory of
stacks (one sub-directory per stack, each with a compose file), shows what is running, and lets the
people you admit start, stop, update and edit those stacks, follow logs, watch stats and open
shells. It runs as one container next to a Docker socket proxy, and everyone signs in through
OIDC, GitHub or Discord.

If you run Dockge today, Depths reads the same stacks directory, so you can try it alongside Dockge
and switch back at any time.

| Guide | What it covers |
|---|---|
| [Installing Depths](install.md) | The example compose file, the socket proxy, the reverse proxy, every setting |
| [Sign-in providers](sign-in.md) | Setting up OIDC, a GitHub OAuth App or a Discord application |
| [People and access](users.md) | The owner, finding your id, admitting people, assigning stacks, sessions and whitelists |
| [Networks and the restricted view](security.md) | Which networks are trusted, what an untrusted one can do, getting back in |
| [Migrating from Dockge](migrating-from-dockge.md) | Pointing Depths at Dockge's directory, what differs, switching back |
| [Tabs, logs and shells](tabs-and-shells.md) | Logs, stats and shell tabs, panes, and what happens when a connection drops |
| [Files in volumes](files.md) | Browsing a stack's volumes, editing config files, transfers, history and limits |

Start with [Installing Depths](install.md), then set up one [sign-in provider](sign-in.md).
