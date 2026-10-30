# Glossary (verified)

This is the starting text for `apps/keel/web/src/help/glossary.ts`. Once that module exists, it is
the single source (FR-024), and this file is history.

Every entry was checked against the server, the protocol crates and the current web UI on
2026-10-29 (T037). Each **Verified** note below names the files that support it. Run
`/impeccable clarify` on the wording (T044) before it ships.

Each entry has at most two sentences and at most 240 characters.

| TermId | Term | Text | Guide |
|---|---|---|---|
| `binding` | Binding | Ties one branch of a repository to a project in a workspace. When the branch moves, Keel fetches it and starts runs for the jobs whose files changed. | `bindings` |
| `receiver` | Receiver | A webhook address with its own secret, shown once. Your Git host calls it on each push, and Keel then fetches the branch itself. | `receivers` |
| `trusted-signers` | Trusted signers | SSH or OpenPGP keys whose signed commits pass the early check without a review. Each host keeps its own list too, so keep the two in step. | `reviews#trusted-signers` |
| `early-check` | Early check | In signed mode, Keel checks a run's new commits against the trusted signers before sending it. Anything not fully signed waits for review, and each host still checks for itself. | `reviews#the-early-check` |
| `review` | Review | A change the early check held back, which someone must approve or deny with a comment. Approving signs it with your passkey, and some changes need more than one approver. | `reviews` |
| `base-profile` | Base profile | The profile from your Ansible repository that every host gets first. Run with an enrolment token, it installs the Keel runner and enrolls the host. | `hosts#run-the-base-profile` |
| `enrolment-token` | Enrolment token | A one-time token that lets one host enroll in a workspace with the profiles you chose. It is shown once and expires after 15 minutes. | `hosts#enroll-a-host` |
| `fingerprint` | Fingerprint | A short code that identifies a key or secret without showing it. Compare it with your own copy to check they match. | `hosts#check-the-fingerprint` |
| `credential` | Credential | A secret a binding uses to fetch the repository or to run its jobs. Keel stores it sealed once you confirm the preview, and from then on shows only its fingerprint. | `bindings#credentials` |
| `inventory-managed` | Managed inventory | An inventory you write and edit in Keel. Each save is a new revision you can compare or restore. | `inventory#managed-and-repository-inventory` |
| `inventory-repository` | Repository inventory | An inventory read from a file in a bound repository. It is read-only here and changes when the repository changes. | `inventory#managed-and-repository-inventory` |
| `workspace` | Workspace | Keeps a set of hosts, bindings, inventories and runs apart from the rest. People get access to it through their team's grants. | `getting-started#create-a-workspace` |
| `team` | Team | A group of people. Grants given to a team apply to all its members. | `getting-started#give-people-access` |
| `grant` | Grant | Gives a team a role in one workspace: viewer, operator, approver or workspace admin. It can be limited to hosts with certain profiles. | `getting-started#give-people-access` |
| `profile` | Profile | A kind of host, declared in the repository's .keel/jobs.yaml. Jobs name the profiles they run on, and a host runs only jobs for profiles it was given. | `hosts#run-the-base-profile` |

Every anchor above must exist as a `##` or `###` heading in that guide (R7). `guide-links.test.ts`
enforces this. On 2026-10-29 only the Settings guides existed under `docs/guides/keel/`, so the
anchors in `bindings`, `receivers`, `reviews`, `hosts`, `inventory` and `getting-started` are
targets for T045–T049 to create, not yet checked.

## Verified

- **`binding`**: corrected. A binding belongs to a project, not straight to a workspace. A push
  does not start a run on every host: Keel fetches, and creates runs only for jobs whose `paths`
  match the changed files.
  `apps/keel/server/src/bindings.rs` (module doc), `apps/keel/server/src/sync.rs`,
  `specs/055-ansible-repo-control-plane/contracts/receivers.md` ("After a sync").
- **`receiver`**: kept, with two additions: the secret is shown once, and Keel fetches the branch
  itself, never trusting the payload's commits.
  `apps/keel/server/src/receivers.rs`, `apps/keel/server/src/hooks.rs`.
- **`trusted-signers`**: reworded. The keys are SSH or OpenPGP. Hosts verify against their own
  `keel_runner_trusted_signers`, and the page already tells people to keep the two lists in step.
  `crates/keel-protocol/src/review.rs` (`SignerInfo`),
  `apps/keel/web/src/features/reviews/ReviewsPage.tsx` (Trusted signers section),
  `specs/055-ansible-repo-control-plane/contracts/host-install.md`.
- **`early-check`**: corrected. It runs only in signed mode and checks the commits between each
  host group's base and the new head. It is advisory: hosts verify on their own.
  `apps/keel/server/src/review.rs` (module doc, "The early check").
- **`review`**: corrected. Denying also needs a comment. A group is approved only at its quorum
  (`override_quorum`), and four-eyes may refuse some people. Approving needs a fresh passkey
  assertion, not just a recent step-up.
  `apps/keel/server/src/review.rs` ("Overrides"), `apps/keel/web/src/features/reviews/ReviewsPage.tsx`.
- **`base-profile`**: corrected. The base profile is not something a host runs after it
  enrolls. It is the operator's Ansible profile, run with the token (`make keel-node` in the
  default snippet), that installs the runner and enrolls the host.
  `specs/055-ansible-repo-control-plane/contracts/host-install.md`, `specs/058-keel-onboard-ansible/spec.md` (FR-005),
  `crates/keel-core/src/enroll_template.rs` (`DEFAULT_TEMPLATE`), `docs/guides/keel/enrollment-template.md`.
- **`enrolment-token`**: kept, made specific. It is for one host (it names the inventory host and
  its profiles) and is valid for 15 minutes.
  `apps/keel/server/src/hosts.rs` (`TOKEN_TTL`, module doc), `crates/keel-protocol/src/host.rs`,
  `apps/keel/web/src/features/hosts/EnrollDialog.tsx`.
- **`fingerprint`**: corrected. Keel has no host-key fingerprint and no compare-before-trust step
  (the 055 join requests are gone). Fingerprints it shows are the SSH CAs' (`SHA256:…`, also
  pinned inside the token), trusted signers' keys, and credentials' (truncated SHA-256 of the
  value).
  `crates/keel-protocol/src/host.rs` (`SshCaInfo`), `crates/keel-protocol/src/review.rs` (`SignerInfo`),
  `crates/keel-protocol/src/binding.rs` (`CredentialInfo`), `apps/keel/web/src/features/hosts/HostsPage.tsx`.
- **`credential`** (was `preview-credential`): corrected, then renamed. There is no read-only
  "preview credential". "Preview credential" is the submit button on the Bindings page's New credential form. Like every
  privileged command, it returns a preview, and the value is stored only on Confirm (create needs
  step-up). See the note under "Open question" below.
  `apps/keel/web/src/features/bindings/BindingsPage.tsx` (`Credentials`), `apps/keel/server/src/credentials.rs`,
  `apps/keel/server/src/preview.rs`.
- **`inventory-managed`**: corrected. "Managed" means edited in Keel (YAML, revisions, restore),
  not hosts that enrolled.
  `crates/keel-protocol/src/inventory.rs` (`InventorySource::Managed`),
  `apps/keel/web/src/features/inventory/InventoryPage.tsx`.
- **`inventory-repository`**: corrected. A repository inventory is a path in a bound repository,
  read-only in Keel and revised by sync. Its hosts are not "unenrolled hosts".
  `crates/keel-protocol/src/inventory.rs` (`InventorySource::Repo`), `apps/keel/server/src/sync.rs`.
- **`workspace`**: reworded. A workspace is the unit of isolation, and access comes through team
  grants, not grants to people.
  `crates/keel-core/src/domain.rs` (`WorkspaceId`), `crates/keel-core/src/authz.rs`.
- **`team`**: unchanged. Access is the union of the grants of a user's teams.
  `crates/keel-core/src/authz.rs`.
- **`grant`**: corrected. A grant goes to a team, never to one person. The roles are viewer,
  operator, approver and workspace admin, and a grant can be scoped to profiles.
  `crates/keel-core/src/authz.rs` (`Grant`, `Capability::of`), `crates/keel-core/src/domain.rs` (`TeamRole`),
  `apps/keel/web/src/features/teams/labels.ts`.
- **`profile`**: corrected. A profile is a kind of host declared in `.keel/jobs.yaml`. Jobs list
  the profiles they run on, and hosts refuse profiles they don't allow. It is not a playbook.
  `specs/055-ansible-repo-control-plane/contracts/repo-manifest.md`, `apps/keel/server/src/sync.rs`,
  `specs/055-ansible-repo-control-plane/spec.md` (FR-037).

## Resolved: credential, not preview credential

`preview-credential` was in FR-020 because the label reads like jargon, but it names an action
(preview, then confirm), not a thing. The owner decided on 2026-10-29 to define `credential`
instead, describing the secret itself; preview is one action that uses it. FR-020 lists
credential, and the tip sits on the Credentials heading of the Bindings page.
