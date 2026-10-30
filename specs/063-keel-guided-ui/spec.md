# Feature Specification: Keel guided UI

**Feature Branch**: `063-keel-guided-ui`

**Created**: 2026-10-29

**Status**: Draft

**Input**: User description: "Keel guided UI (second user critique, 2026-10-29). Three changes to the Keel web UI, keeping the dense, mobile-first neon-noir look, the existing type tokens, and sentence case. (1) Grouped navigation under three always-open headed groups: Pipelines (Runs, Reviews, Bindings, Receivers), Infrastructure (Inventory, Hosts), Administration (Teams, Users, Audit, Migration); System Settings stays pinned at the bottom; on a phone one scrolling strip with dividers. (2) Guided empty states: centred primary action, a one-line reason, two or three numbered next steps, copy buttons for values that must be pasted elsewhere, a link to the matching guide, no illustrations; people who can't act see who can. (3) Help for jargon: info icons with short tooltips from one glossary, reachable by keyboard and touch; long paragraphs become a subtitle plus 'Learn more'. Out of scope: table column changes, collapsible nav, illustrations, server or protocol changes."

## Why

Keel's second outside critique (2026-10-29) found three problems that the first polish pass (sentence case, type scale, button weights) didn't touch:

- **A flat navigation.** Ten sections sit in one list. Deployment work (Runs, Reviews), the wiring that feeds it (Bindings, Receivers), the machines (Inventory, Hosts) and administration (Teams, Users, Audit) all look alike, so a newcomer can't see how Keel fits together.
- **Dead-end empty states.** A new Keel is mostly empty lists, and each one says what to do in a sentence ("No receivers yet. Create one, then add its URL and secret at your Git host") with the action in a toolbar elsewhere. The values a person has to paste into another tool aren't offered where they are needed. Even the owner, on first sign-in, sees "You have no workspace yet. An owner or admin grants one from Teams" without a way to do it.
- **Unexplained jargon.** Terms like trusted signers, base profile, fingerprint and preview credential appear without explanation, and where Keel does explain, it does so in dense paragraphs.

The owner's direction is to adapt to users. This feature makes the first hour with Keel self-guiding without changing what Keel does.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A first-time owner sets Keel up from inside Keel (Priority: P1)

An owner signs in to a new, empty Keel. Every empty page tells them, in place, what the page is for, what to do first, and gives them the button to do it. Values they must take elsewhere (a receiver's URL and secret, a host's enroll command) come with copy buttons, next to a link to the guide for that step.

**Why this priority**: This is where the critique hurt most, and it's the path every new install takes. The owner who reported the confusion was stuck at "You have no workspace yet" while being the owner.

**Independent Test**: Start from an empty Keel as an owner. Create a workspace, enroll a host and create a receiver using only the guidance on the empty pages, leaving Keel only to paste values into a Git host and run the enroll command on the host.

**Acceptance Scenarios**:

1. **Given** an owner or admin with no workspace, **When** they reach a page that needs one, **Then** they see why a workspace is needed and a "Create a workspace" action that takes them to the place to create it, not a message telling them to ask an owner or admin.
2. **Given** a user (not owner or admin) with no workspace, **When** they reach the same page, **Then** they see that an owner or admin grants workspaces from Teams, and no action they can't take.
3. **Given** a workspace with no hosts, **When** a person who may enroll hosts opens Hosts, **Then** the page centres an "Enroll a host" action with a one-line reason and numbered steps (enroll, run the command on the host, run the base profile), and a link to the hosts guide.
4. **Given** the enroll dialog has produced a token and command, **When** the person looks at it, **Then** each value they must paste elsewhere has its own copy button.
5. **Given** a workspace with no receivers, **When** a person who may create one opens Receivers, **Then** the page centres "Create a receiver" with the steps (create it, add its URL and secret at the Git host, push), and after creation the URL and secret each have a copy button.
6. **Given** any page in scope (listed in FR-010) with an empty main list, **When** a person who can't take the primary action opens it, **Then** they see what the page is for and which role can act, and no disabled or missing-looking button.
7. **Given** a list that is empty only because of a filter, **When** the page shows it, **Then** it shows the plain "nothing matches" line with a way to clear the filter, not the guided empty state.

---

### User Story 2 - The navigation shows how Keel fits together (Priority: P2)

The sections are grouped under three headings: Pipelines (Runs, Reviews, Bindings, Receivers), Infrastructure (Inventory, Hosts) and Administration (Teams, Users, Audit, Migration). The groups are always open. System Settings stays pinned at the bottom for owners and admins.

**Why this priority**: It's cheap and helps everyone on every visit, but it doesn't unblock anyone the way US1 does.

**Independent Test**: Open Keel at desktop width and see three headed groups in the stated order with the stated sections; open it at 360 px and see one scrolling strip with a divider between the groups and no new horizontal page scroll.

**Acceptance Scenarios**:

1. **Given** a desktop-width window, **When** the shell renders, **Then** the sidebar shows the three group headings in order, each followed by its sections in the stated order, and System Settings at the bottom for owners and admins.
2. **Given** a phone-width window, **When** the shell renders, **Then** the sections form one horizontally scrolling strip in the same order, with a visible divider between groups and no group headings.
3. **Given** a person for whom every section of a group is hidden, **When** the shell renders, **Then** that group's heading and divider are not shown.
4. **Given** a screen reader, **When** it reads the navigation, **Then** each group is announced with its name, and the current section is still marked as current.
5. **Given** the current section, **When** the shell renders, **Then** it keeps the existing current-item treatment (the accent stays reserved for the current item), and group headings never use the accent colour.

---

### User Story 3 - Jargon explains itself (Priority: P3)

Next to the terms Keel's newcomers don't know, an info icon opens a one- or two-sentence explanation. The same term reads the same everywhere because the text comes from one glossary. Long explanatory paragraphs under page and section headings become a one-line subtitle with "Learn more" linking to the guide.

**Why this priority**: It helps understanding, but people can already work without it, and US1's steps already explain the first-run terms in context.

**Independent Test**: Open each page that shows a glossary term (FR-020) and confirm the icon opens the glossary text by mouse hover, by keyboard focus and by a tap; confirm two places that show the same term show the same text.

**Acceptance Scenarios**:

1. **Given** a glossary term in a page heading, section heading, table column heading or form label, **When** the page renders, **Then** an info icon sits right after the term.
2. **Given** the info icon, **When** a person hovers it, focuses it with the keyboard, or taps it, **Then** the explanation appears; Escape or tapping elsewhere closes it.
3. **Given** a screen reader on the icon, **When** it is focused, **Then** it is announced as help for that term, and the explanation is read.
4. **Given** a section whose explanation today is longer than one line (for example trusted signers on Reviews), **When** it renders, **Then** it shows a one-line subtitle and a "Learn more" link to the guide section, and no paragraph.
5. **Given** a phone-width window, **When** an explanation opens, **Then** it fits on screen without horizontal scrolling.

### Edge Cases

- **A page that is loading or failed to load** keeps its loading and error states; the guided empty state appears only once the list has loaded and is empty.
- **A list emptied by a filter or search** shows the plain "nothing matches" line (US1 scenario 7).
- **A workspace a person can see but not change** (for example a viewer) shows the explanation and the role that can act, never a dead button.
- **A secondary list on a page** (deliveries under a receiver, revisions of an inventory, hosts reporting on one run) keeps a one-line empty message; guided empty states are for the page's main list only.
- **Very narrow screens (360 px)**: centred actions, numbered steps, copy rows and explanations wrap; long values (URLs, secrets, commands) scroll inside their own box, never the page.
- **A secret that is shown only once** (a receiver's secret, an enroll token) keeps its "shown once" warning next to the copy button.
- **Guides that don't exist yet**: a "Learn more" or guide link appears only where the linked guide exists in the published guides; this feature adds the guides it links to (FR-030).
- **The Migration section** has no empty list to guide; it only moves into the Administration group.

## Requirements *(mandatory)*

### Functional Requirements

**Grouped navigation (US2)**

- **FR-001**: The section navigation MUST show three groups in this order: Pipelines (Runs, Reviews, Bindings, Receivers), Infrastructure (Inventory, Hosts), Administration (Teams, Users, Audit, Migration).
- **FR-002**: On wide screens each group MUST have a visible heading in sentence case, styled as a label, never in the accent colour. Groups MUST NOT collapse.
- **FR-003**: On narrow screens the navigation MUST stay one horizontally scrolling strip in the same order, with a divider between groups and no headings.
- **FR-004**: A group with no section visible to the person MUST be hidden with its heading and divider.
- **FR-005**: System Settings MUST stay pinned at the bottom (last in the strip on a phone), for owners and admins only, with its update dot unchanged.
- **FR-006**: Assistive technology MUST announce each group by name; the current section MUST stay marked as current.

**Guided empty states (US1)**

- **FR-010**: These pages MUST show a guided empty state when their main list has loaded and is empty: Runs, Reviews (the queue), Bindings, Receivers, Inventory, Hosts, Teams (workspaces and teams), and the workspace picker's "no workspace" state.
- **FR-011**: A guided empty state MUST show, centred in the page's content area: a short title, one line on why the thing matters, two or three numbered next steps, the page's primary action as the one primary button, and a link to the matching guide.
- **FR-012**: When the person can't take the primary action, the empty state MUST show the title, the reason and the role that can act, and MUST NOT show the action button.
- **FR-013**: When the person has no workspace, owners and admins MUST be offered "Create a workspace", taking them to where workspaces are created; other users MUST be told that an owner or admin grants workspaces from Teams.
- **FR-014**: A list empty only because of a filter or search MUST show "nothing matches" with a way to clear it, not the guided empty state.
- **FR-015**: Wherever the person must paste a value Keel shows into another tool (a receiver's URL and secret, a host's enroll token and command), each such value MUST have its own copy button with a visible confirmation that it was copied. A value shown only once MUST keep its warning.
- **FR-016**: Secondary lists inside a page MUST keep a one-line empty message.
- **FR-017**: Guided empty states MUST NOT use illustrations.

**Help for jargon (US3)**

- **FR-020**: A single glossary MUST define at least: binding, receiver, trusted signers, early check, review, base profile, enroll / enrolment token, fingerprint, credential, inventory (managed and repository), workspace, team, grant, profile. Each entry is one or two sentences and may name a guide section.
- **FR-021**: Where a glossary term appears in a page heading, section heading, table column heading or form label, an info icon MUST follow it, opening the glossary text. Each term gets an icon at most once per section.
- **FR-022**: The explanation MUST open on hover, on keyboard focus and on tap; it MUST close on Escape, on blur and on tapping elsewhere; it MUST be announced to assistive technology as help for the term.
- **FR-023**: Explanatory text under page and section headings longer than one line at desktop width MUST become a one-line subtitle plus a "Learn more" link to the guide.
- **FR-024**: The glossary MUST be the only source of tooltip text: the same term MUST read the same everywhere.

**Guides and cross-cutting**

- **FR-030**: Every guide link and "Learn more" link MUST point at a guide that exists. This feature MUST add the guides it links to and that don't exist yet, under the Keel user guides: getting started (workspace → host → receiver → binding → first run), hosts and enrolment, receivers, bindings and credentials, reviews and trusted signers, inventory.
- **FR-031**: All new text MUST be in sentence case and use the existing type tokens; the accent colour stays reserved for primary actions and the current navigation item.
- **FR-032**: At 360 px, no page in scope MUST scroll horizontally because of this feature.
- **FR-033**: This feature MUST NOT change the server, the protocol, permissions or table columns.

### Key Entities

- **Navigation group**: a name and an ordered list of sections. Visible when at least one of its sections is.
- **Guided empty state**: for one page's main list: a title, a one-line reason, two or three steps, the primary action and who may take it, and a guide link.
- **Glossary entry**: a term, a one- or two-sentence explanation, and an optional guide section link.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A first-time owner goes from an empty Keel to a workspace, an enrolled host and a receiver that has received a delivery, leaving Keel only to paste into the Git host and to run the enroll command on the host, in under 15 minutes.
- **SC-002**: 100% of the pages in FR-010 show a guided empty state with an action or the role that can act; none shows a dead-end sentence.
- **SC-003**: 100% of the glossary terms in FR-020 that appear in headings, column headings or labels have an info icon, and the icon works by mouse, keyboard and touch.
- **SC-004**: Every value a person must paste elsewhere during the SC-001 walk-through is one click to copy.
- **SC-005**: At 360 px and at 1280 px, in light and dark themes, no page in scope scrolls horizontally, and the existing layout checks still pass.
- **SC-006**: Every guide link resolves to an existing guide.

## Assumptions

- Group membership follows the critique's grouping, with Migration added to Administration (the critique listed nine sections; Keel has ten).
- Which roles can take each primary action is unchanged: the guided empty state reads the same permission the page already uses.
- The values to copy already exist in Keel's responses (receiver URL and secret, enroll token and command); the feature only offers them better.
- Owners and admins may create workspaces today; "Create a workspace" goes to the existing place for that.
- The new guides are short procedural pages in the existing Keel guides folder, written in the guides' current voice, with screenshots taken last.
- Table column changes, collapsible navigation and illustrations were offered and declined by the owner (2026-10-29).
- Depths and Fathom are unaffected unless a shared component changes; any shared change keeps their behaviour.
