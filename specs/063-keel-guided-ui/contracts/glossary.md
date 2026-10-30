# Glossary (draft)

This is the starting text for `apps/keel/web/src/help/glossary.ts`. Once that module exists, it is
the single source (FR-024), and this file is history.

**Check every entry against the server's behaviour before committing it.** Rewording is expected:
fit the facts, then run `/impeccable clarify`.

Each entry has at most two sentences and at most 240 characters.

| TermId | Term | Text | Guide |
|---|---|---|---|
| `binding` | Binding | Connects a repository branch to a workspace. A push to that branch starts a run on the workspace's hosts. | `bindings` |
| `receiver` | Receiver | A webhook address with its own secret. Your Git host calls it on every push, so Keel knows a branch moved. | `receivers` |
| `trusted-signers` | Trusted signers | Signing keys whose commits skip review. The early check compares each commit's signature against this list. | `reviews#trusted-signers` |
| `early-check` | Early check | Keel's check of a commit before it runs. Commits signed by a trusted signer pass; others wait for review. | `reviews#the-early-check` |
| `review` | Review | A person approves or denies a change before Keel runs it. Approving asks for your passkey. | `reviews` |
| `base-profile` | Base profile | The first profile a newly enrolled host runs. It sets the host up so later runs can manage it. | `hosts#run-the-base-profile` |
| `enrolment-token` | Enrolment token | A one-time token that lets a host join a workspace. It is shown once and expires. | `hosts#enroll-a-host` |
| `fingerprint` | Fingerprint | A short digest of a host's key. Compare it with the host's own output before you trust it. | `hosts#check-the-fingerprint` |
| `preview-credential` | Preview credential | A read-only credential Keel uses to preview a change before it is approved. | `bindings#credentials` |
| `inventory-managed` | Managed inventory | Hosts that enrolled in Keel and report back to it. | `inventory#managed-and-repository-inventory` |
| `inventory-repository` | Repository inventory | Hosts listed in the bound repository's own inventory files. Keel shows them, but they haven't enrolled. | `inventory#managed-and-repository-inventory` |
| `workspace` | Workspace | A space that holds hosts, bindings and runs. People get access to it through grants. | `getting-started#create-a-workspace` |
| `team` | Team | A group of people. Grants given to a team apply to all its members. | `getting-started#give-people-access` |
| `grant` | Grant | Gives a person or team a role in one workspace, such as viewer, operator or workspace admin. | `getting-started#give-people-access` |
| `profile` | Profile | A named playbook from the bound repository that Keel can run on a host. | `hosts#run-the-base-profile` |

Every anchor above must exist as a `##` or `###` heading in that guide (R7). `guide-links.test.ts`
enforces this.
