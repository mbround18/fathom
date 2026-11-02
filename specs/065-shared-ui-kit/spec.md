# Feature Specification: Shared UI kit

**Feature Branch**: `065-shared-ui-kit`

**Created**: 2026-11-01

**Status**: Draft

**Input**: User description: "Take all we did with Keel and churn them into a ./packages/ui that
houses all our shared assets and components so each UI shares a similar look and feel and
functionality." Expanded: move the screen-level patterns built and polished in Keel (specs 055,
063, 064) into the existing shared package `packages/ui`, so Keel, Depths and the Fathom desktop
app share one look, feel and behaviour. Behaviour must not change for Keel. Depths adopts the
shared pieces where it has equivalents. Fathom adopts the leaf pieces and keeps its pane layout.
App-specific content stays in the apps. No new dependencies. Out of scope: redesigning the look,
changing the theme tokens, and suite mode.

## Why

The three apps already share low-level parts: buttons, inputs, menus, the theme and the brand
marks. What they don't share is how a screen behaves. Keel worked out, through three specs and an
owner's critique, how to answer these questions:

- what a page title and its sections look like;
- what an empty list says, and how it guides someone to fill it;
- what a list that failed to load says: "Failed to load {what}.", the reason, and Retry;
- what happens when a screen crashes: the rest of the app keeps working, and a Reload is offered;
- how a change is previewed, then confirmed, with a passkey when needed;
- how navigation is grouped, and how a count or an update shows on a menu item;
- how jargon gets a short explanation in place.

Depths and Fathom answer these questions differently, or not at all. Someone who uses two of the
apps meets two ways of saying the same thing. Each fix (the owner's "Failed to load" request is
the latest) has to be made three times, or it reaches only one app.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Keel keeps working, from shared parts (Priority: P1)

Keel's generic screen pieces move into the shared package. Keel then uses them from there. A Keel
user notices no change: every page looks and behaves exactly as before.

**Why this priority**: the other apps can't adopt a piece until it exists in the shared package.
Keel is the source and the proof that nothing broke.

**Independent Test**: Keel's full unit and browser suites pass, with only import paths changed.
Screenshots of Keel's screens at 360 and 1280 px, light and dark, match the screenshots taken
before the move.

**Acceptance Scenarios**:

1. **Given** the shared package holds the page, empty, failed-to-load, crash, dialog, table,
   copy, form, tabs, help and navigation pieces, **When** Keel is built, **Then** Keel has no
   copy of any of them in its own code.
2. **Given** any Keel screen, **When** it is compared with its screenshot from before the move,
   **Then** there is no visible difference.
3. **Given** a list that fails to load in Keel, **When** it is shown, **Then** it reads "Failed
   to load {what}." with the reason and Retry, exactly as now.

---

### User Story 2 - Depths looks and behaves like Keel (Priority: P2)

A Depths user sees the same page titles, empty states, failed-to-load states, crash handling,
confirm dialogs, tables and grouped navigation as Keel. Depths keeps its own content: stacks,
files, the editor and terminal, sessions, passkeys and updates.

**Why this priority**: Depths is a web app with the same shape as Keel: a navigation shell and
routed pages. It gains the most, at the least risk.

**Independent Test**: Depths' unit and browser suites pass. A list that fails to load in Depths
shows "Failed to load {what}." with the reason and Retry. A screen that throws shows the crash
message while the navigation keeps working.

**Acceptance Scenarios**:

1. **Given** a Depths list whose load fails, **When** the page shows, **Then** it reads "Failed
   to load {what}." with the reason, and Retry loads it again.
2. **Given** a Depths page that throws while rendering, **When** someone opens it, **Then** they
   see "Something went wrong showing this page." with Reload, and can still use the navigation.
3. **Given** an empty Depths list that someone can fill, **When** it shows, **Then** it says what
   belongs there, the steps to add the first one and, where one exists, a link to the guide.
4. **Given** a Depths change that needs confirmation, **When** someone starts it, **Then** the
   shared preview-then-confirm dialog opens, with a passkey prompt where the server asks for one.

---

### User Story 3 - Fathom uses the same leaf pieces (Priority: P3)

The Fathom desktop app keeps its own layout: panes, tabs, the command palette and the phone
stack. Inside a pane, it uses the shared pieces for empty lists, failed loads, crashes, confirm
dialogs and copyable text.

**Why this priority**: Fathom's layout is unlike the web apps', and it has the largest suite. So
it adopts only the pieces that fit inside its panes.

**Independent Test**: Fathom's unit and browser suites pass. A pane whose load fails shows the
shared failed-to-load state. A pane that throws shows the crash state inside that pane only, and
the other panes keep working.

**Acceptance Scenarios**:

1. **Given** a Fathom pane whose list fails to load, **When** it shows, **Then** it reads "Failed
   to load {what}." with the reason and Retry.
2. **Given** a Fathom pane that throws, **When** it renders, **Then** that pane alone shows the
   crash state, and the other panes keep working.
3. **Given** a Fathom action that asks for confirmation, **When** it opens, **Then** the dialog
   is the shared confirm dialog.

---

### User Story 4 - A developer builds a new screen from the kit (Priority: P3)

Someone adding a screen to any of the three apps finds every shared piece in one place. Each
piece has a short note on when to use it. They build the screen without copying code from
another app.

**Why this priority**: the kit pays off only if new screens use it. Without a guide, copies
creep back in.

**Independent Test**: the contributor guide lists every shared piece with one line on when to
use it. A check fails when an app holds its own copy of a piece that the kit provides.

**Acceptance Scenarios**:

1. **Given** the contributor guide, **When** a developer looks for how to show an empty list,
   **Then** they find the shared piece and an example of its use.
2. **Given** an app that adds its own copy of a shared piece, **When** the checks run, **Then**
   they fail and name the shared piece to use instead.

### Edge Cases

- A piece that Keel ties to its own data, such as the workspace picker, the protocol's refusal
  shape or the glossary text, stays in Keel. The shared piece takes that data from the app.
- Text that differs by app, such as guide links and "who can" wording, comes from the app, never
  from the kit.
- Depths or Fathom has a similar piece that behaves differently from Keel's. The shared piece
  follows Keel's behaviour. Each such difference is listed and called out as a visible change.
- An app has no help guides. The help pieces still work, and simply show no guide link.
- Tests that find elements by their markers (the `data-` attributes) keep finding them: the
  shared pieces keep the markers Keel uses today.
- The kit must not grow a dependency on any one app's server contract.

## Requirements *(mandatory)*

### Functional Requirements

**The kit**

- **FR-001**: The shared package MUST provide one piece for each of these patterns:
  - the page frame, and the page header with the title scale;
  - the guided empty state, and the choice between guided, plain and filtered-out;
  - "no match" for a filter that hides everything;
  - loading;
  - "Failed to load {what}." with the reason and Retry, and the helper that loads a list into
    that state;
  - the screen crash boundary;
  - the inline error line;
  - the data table;
  - the dialog, and preview-then-confirm with its preview details;
  - copyable text, and copying to the clipboard;
  - form styling: field, label, link and heading styles, with inputs capped at about 40rem;
  - tabs;
  - Markdown rendering;
  - the guide link, and the term explanation popover;
  - the navigation shell with section headings, the count badge and the update dot;
  - in-app links, routing and the query-parameter hook, if they carry no Keel-specific
    behaviour.
- **FR-002**: Each shared piece MUST take app-specific content from the app that uses it: text,
  guide addresses, glossary terms, the refusal shape and permission checks. The kit holds no
  app's words or rules.
- **FR-003**: Each shared piece MUST keep the visible output, the accessible names and roles, and
  the test markers of the Keel piece it replaces.
- **FR-004**: The kit MUST NOT add a new third-party dependency to the workspace.
- **FR-005**: Brand marks and icons MUST stay in the branding package, and colour and type tokens
  in the theme package. The kit uses them and does not copy them.

**Keel**

- **FR-006**: Keel MUST use the shared pieces and keep no copies of them.
- **FR-007**: Keel's screens MUST look and behave as they did before the move, at 360 and
  1280 px, in light and dark.

**Depths**

- **FR-008**: Depths MUST use the shared navigation shell, page header, empty, failed-to-load,
  crash, dialog, confirm and table pieces wherever it shows those patterns.
- **FR-009**: Every Depths list that loads from the server MUST show the shared failed-to-load
  state when the load fails.
- **FR-010**: Depths MUST have a crash boundary around each routed page.
- **FR-011**: Any visible change that adoption causes in Depths MUST be listed in the change
  notes for this feature.

**Fathom**

- **FR-012**: Fathom MUST use the shared empty, failed-to-load, crash, confirm-dialog and
  copyable-text pieces inside its panes. Its pane layout, tabs and navigation stay as they are.
- **FR-013**: A crash inside one Fathom pane MUST NOT take down the other panes.

**Keeping it shared**

- **FR-014**: The contributor guide MUST list each shared piece, with when to use it and a short
  example.
- **FR-015**: A check MUST fail when an app defines its own piece with the same name as one in
  the kit, unless the app's piece is on a short allow-list with a reason.
- **FR-016**: The shared pieces MUST have their own unit tests in the shared package. The apps'
  tests cover how each app uses them.

### Key Entities

- **Shared piece**: a screen pattern held once in the shared package. It has a name, the content
  it takes from the app, and the test markers it keeps.
- **App content**: the words, links, terms and rules one app hands to a shared piece.
- **Allow-listed copy**: an app-local piece that keeps a kit name on purpose, with a written
  reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Each pattern listed in FR-001 exists exactly once across the repository, in the
  shared package. Allow-listed copies are the only exceptions.
- **SC-002**: Keel's, Depths' and Fathom's existing unit and browser tests all pass. Keel's tests
  change only in import paths.
- **SC-003**: Keel's before-and-after screenshots, at 360 and 1280 px in light and dark, show no
  visible difference on any screen.
- **SC-004**: In Depths and in Fathom, every list that loads from the server shows "Failed to
  load {what}." with the reason and Retry when the load fails: 100% of such lists, checked by a
  browser test per app.
- **SC-005**: A screen that crashes in any of the three apps shows the crash state and leaves the
  rest of the app usable. Each app has a browser test for this.
- **SC-006**: A developer can find the right piece for an empty list, a failed load and a
  confirmation from the contributor guide in under 2 minutes.

## Assumptions

- **The package is the existing one.** "./packages/ui" is the existing shared package
  (`@companion/ui`), which all three apps already use for buttons, inputs and menus. No new
  package is created.
- **Keel's look is the reference.** Where Depths or Fathom differ from Keel, the shared piece
  follows Keel. The owner chose Keel's polish in spec 063 and its critique pass.
- **Fathom keeps its layout.** Fathom's panes, tabs, command palette and phone stack are not
  replaced. Its navigation stays its own.
- **Help content stays per app.** The guide and glossary machinery is shared. Each app's guides
  and terms stay with that app, and an app may have none.
- **Routing moves only if it is generic.** Keel's and Depths' small routers are expected to be
  near twins. If either holds app-specific behaviour, that part stays in the app.
- **No server changes.** This is a web-side change. Each app's refusal or error shape is mapped
  to the kit's by that app.
- **Suite mode is not part of this.** Running the apps as one product stays in the later
  "marry them" spec. This spec only makes them look and behave alike.
- **The brochure screenshots for spec 063 (T054) are skipped** for now, at the owner's request
  (2026-11-01).
