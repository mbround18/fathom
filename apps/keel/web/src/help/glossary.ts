/**
 * The glossary (spec 063 R5, FR-020, FR-024): the only source of the text an info icon shows, so a
 * term reads the same on every page. `<TermTip term="…" />` looks a term up here.
 *
 * Each entry is at most two sentences and 240 characters, in sentence case, and may point at the
 * guide section that explains it in full (`tests/unit/glossary.test.ts` holds it to that). The
 * wording was checked against the server on 2026-10-29: specs/063-keel-guided-ui/contracts/glossary.md
 * lists the files behind each entry. Change the facts here only when the server's behaviour changes.
 */
import type { GuideRef } from './guides'

export type TermId =
  | 'binding'
  | 'receiver'
  | 'trusted-signers'
  | 'early-check'
  | 'review'
  | 'base-profile'
  | 'enrolment-token'
  | 'fingerprint'
  | 'credential'
  | 'inventory-managed'
  | 'inventory-repository'
  | 'workspace'
  | 'team'
  | 'grant'
  | 'profile'

export interface GlossaryEntry {
  /** The term as a heading shows it, in sentence case; the icon reads "About {term}". */
  term: string
  /** One or two sentences. */
  text: string
  guide?: GuideRef
}

export const GLOSSARY: Readonly<Record<TermId, GlossaryEntry>> = {
  binding: {
    term: 'Binding',
    text: 'Ties one branch of a repository to a project in a workspace. When the branch moves, Keel fetches it and starts runs for the jobs whose files changed.',
    guide: { guide: 'bindings' },
  },
  receiver: {
    term: 'Receiver',
    text: 'A webhook address with its own secret, shown once. Your Git host calls it on each push, and Keel then fetches the branch itself.',
    guide: { guide: 'receivers' },
  },
  'trusted-signers': {
    term: 'Trusted signers',
    text: 'SSH or OpenPGP keys whose signed commits pass the early check without a review. Each host keeps its own list too, so keep the two in step.',
    guide: { guide: 'reviews', anchor: 'trusted-signers' },
  },
  'early-check': {
    term: 'Early check',
    text: "In signed mode, Keel checks a run's new commits against the trusted signers before sending it. Anything not fully signed waits for review, and each host still checks for itself.",
    guide: { guide: 'reviews', anchor: 'the-early-check' },
  },
  review: {
    term: 'Review',
    text: 'A change the early check held back, which someone must approve or deny with a comment. Approving signs it with your passkey, and some changes need more than one approver.',
    guide: { guide: 'reviews' },
  },
  'base-profile': {
    term: 'Base profile',
    text: 'The profile from your Ansible repository that every host gets first. Run with an enrolment token, it installs the Keel runner and enrolls the host.',
    guide: { guide: 'hosts', anchor: 'run-the-base-profile' },
  },
  'enrolment-token': {
    term: 'Enrolment token',
    text: 'A one-time token that lets one host enroll in a workspace with the profiles you chose. It is shown once and expires after 15 minutes.',
    guide: { guide: 'hosts', anchor: 'enroll-a-host' },
  },
  fingerprint: {
    term: 'Fingerprint',
    text: 'A short code that identifies a key or secret without showing it. Compare it with your own copy to check they match.',
    guide: { guide: 'hosts', anchor: 'check-the-fingerprint' },
  },
  credential: {
    term: 'Credential',
    text: 'A secret a binding uses to fetch the repository or to run its jobs. Keel stores it sealed once you confirm the preview, and from then on shows only its fingerprint.',
    guide: { guide: 'bindings', anchor: 'credentials' },
  },
  'inventory-managed': {
    term: 'Managed inventory',
    text: 'An inventory you write and edit in Keel. Each save is a new revision you can compare or restore.',
    guide: { guide: 'inventory', anchor: 'managed-and-repository-inventory' },
  },
  'inventory-repository': {
    term: 'Repository inventory',
    text: 'An inventory read from a file in a bound repository. It is read-only here and changes when the repository changes.',
    guide: { guide: 'inventory', anchor: 'managed-and-repository-inventory' },
  },
  workspace: {
    term: 'Workspace',
    text: "Keeps a set of hosts, bindings, inventories and runs apart from the rest. People get access to it through their team's grants.",
    guide: { guide: 'getting-started', anchor: 'create-a-workspace' },
  },
  team: {
    term: 'Team',
    text: 'A group of people. Grants given to a team apply to all its members.',
    guide: { guide: 'getting-started', anchor: 'give-people-access' },
  },
  grant: {
    term: 'Grant',
    text: 'Gives a team a role in one workspace: viewer, operator, approver or workspace admin. It can be limited to hosts with certain profiles.',
    guide: { guide: 'getting-started', anchor: 'give-people-access' },
  },
  profile: {
    term: 'Profile',
    text: "A kind of host, declared in the repository's .keel/jobs.yaml. Jobs name the profiles they run on, and a host runs only jobs for profiles it was given.",
    guide: { guide: 'hosts', anchor: 'run-the-base-profile' },
  },
}
