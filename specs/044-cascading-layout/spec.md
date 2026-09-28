# Feature Specification: Cascading wide-screen layout

**Feature Branch**: `044-cascading-layout`

**Created**: 2026-09-27

**Status**: Draft

**Input**: User description: "Cascading wide-screen layout (feedback Group 3: items #6, #7, #8). On wide and ultra displays, slot 1 is the sections navigator, slot 2 is the selected section's list, and slots 3+ stay empty until the user drills in, then fill as they cascade. Clicking a section auto-opens its most relevant item (e.g. the first unhealthy resource) in slot 3; further slots fill only as the user clicks. The current wide/ultra profiles prefill extra list, Overview and Flux slots, which is the opposite of this. Small windows (phone, tablet, laptop) keep today's behaviour exactly. Decisions already made by the user are recorded in docs/FEEDBACK_2026-09-27.md under 'Group 3: cascading layout'."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A wide screen opens on one thing, not five (Priority: P1)

An operator with a 1920 px or wider window connects to a cluster. The leftmost column is the
sections navigator. They click a section — say Workloads → Pods. The second column fills with that
section's list, and the third column opens the item in that list that most wants attention: the
first unhealthy resource if there is one, otherwise the first row. Every column to the right of
that is visibly empty and labelled as waiting for the next click. Nothing on screen belongs to a
section the operator did not ask for.

**Why this priority**: This is the whole complaint (feedback #6/#7): today a wide window opens with
a second resource list, an Overview and a Flux pane the operator never asked for, so the screen
reads as noise before any work starts. Fixing the initial state alone makes wide displays usable.

**Independent Test**: Open the app at wide and at ultra width against a scripted backend, click one
section, and assert exactly three occupied columns (navigator, list, the auto-opened item) with the
remaining columns showing the empty placeholder.

**Acceptance Scenarios**:

1. **Given** a wide window and a connected context, **When** the operator clicks a section in the
   navigator, **Then** column 2 shows that section's list, column 3 shows the details of that
   section's most relevant item, and columns 4 and beyond are empty placeholders.
2. **Given** an ultra window (five columns) and a connected context, **When** the operator clicks a
   section, **Then** columns 4 and 5 are empty placeholders rather than an Overview and a Flux pane.
3. **Given** a section whose list has at least one unhealthy item, **When** the operator clicks that
   section, **Then** the item opened in column 3 is the first unhealthy item in the list's order.
4. **Given** a section whose list is entirely healthy, **When** the operator clicks that section,
   **Then** the item opened in column 3 is the first row of the list.
5. **Given** a section whose list is empty, or a section that has no list (for example a settings or
   topology destination), **When** the operator clicks it, **Then** column 2 shows that destination
   and column 3 stays an empty placeholder.

---

### User Story 2 - Drilling in cascades to the right (Priority: P2)

From the auto-opened item in column 3, the operator follows a relationship — a Deployment to its
ReplicaSet, a ReplicaSet to a Pod, a Pod to a container. Each click opens the new thing in the next
empty column to the right, leaving the chain that led there visible. When the columns run out, the
chain scrolls: the oldest cascade column is dropped, the rest shift left, and the newest opens on
the right. The navigator and the section list never move.

**Why this priority**: Cascading is what makes the extra width pay for itself (feedback #7, "spread
the next 3 or 4 levels across it automatically"). It depends on US1 having cleared the prefilled
slots, so it ships second.

**Independent Test**: With the scripted backend, click through a four-level chain at ultra width and
assert each step lands in the next column, then assert a fifth step shifts the chain left while
columns 1 and 2 are unchanged.

**Acceptance Scenarios**:

1. **Given** a cascade occupying columns 1–3 at wide width, **When** the operator follows a link
   from column 3, **Then** the target opens in column 4 and column 3 still shows where they came
   from.
2. **Given** every column occupied, **When** the operator follows one more link, **Then** the new
   target opens in the rightmost column, the intermediate columns shift one place left, the oldest
   drill-down leaves the screen, and columns 1 and 2 still show the navigator and the section list.
3. **Given** a cascade three levels deep, **When** the operator clicks a different row in the
   section list in column 2, **Then** column 3 shows that row and columns 4 and beyond return to
   empty placeholders.
4. **Given** a cascade of any depth, **When** the operator clicks a different section in the
   navigator, **Then** the cascade resets to the US1 state for that section.

---

### User Story 3 - Small windows are untouched (Priority: P2)

An operator on a laptop, a tablet or a phone sees exactly the behaviour they see today: the phone
drills in as a stack, the tablet keeps its list-and-inspector split, the laptop keeps its
navigator, list and inspector.

**Why this priority**: Explicitly requested (feedback #8, and "the current small-window layout" is
on the keep-as-is list). It is a regression guard rather than new value, so it has no independent
value of its own — but it must hold from the first merge, so it shares P2.

**Independent Test**: Run the existing phone, tablet and laptop end-to-end suites unchanged and
assert they still pass, plus an explicit assertion that a laptop window's inspector waits for the
operator's click, as it does today.

**Acceptance Scenarios**:

1. **Given** a phone window, **When** the operator taps a row, **Then** navigation stacks and pops
   exactly as it does before this change.
2. **Given** a laptop or desktop window, **When** the operator clicks a section, **Then** the three
   columns behave as they do before this change: the inspector stays empty until the operator picks
   a row (there is no auto-open below wide width).
3. **Given** a tablet window, **When** the operator selects a row, **Then** the inspector shows it
   as it does before this change.

---

### User Story 4 - The empty columns invite the next click (Priority: P3)

The columns that are waiting are not blank rectangles: each says what will appear there ("Follow a
link to open the next level here") so a newcomer understands the cascade instead of reading empty
space as a broken layout. The placeholder is the same width as an occupied column, so opening one
does not reflow the columns already on screen.

**Why this priority**: Feedback #10 ("the UI feels chaotic … so someone new to Kubernetes can find
their way around") applies to empty space too, but the layout is correct without it.

**Independent Test**: At wide width with one section open, assert the empty columns carry visible
explanatory text, and assert the widths of columns 1–3 are unchanged after column 4 fills.

**Acceptance Scenarios**:

1. **Given** a wide window with columns 4 and beyond empty, **When** the operator looks at them,
   **Then** each shows a short explanation of what a click will put there.
2. **Given** the operator has resized the columns by dragging, **When** an empty column later
   fills, **Then** the sizes they chose are kept.

---

### Edge Cases

- **A saved layout from before this change** names prefilled panes (a second list, Overview, Flux)
  in its wide or ultra profile. The saved file must keep loading, and the operator must end up in
  the new clean state rather than with the old prefill restored.
- **Width class changes while a cascade is open** (the operator resizes the window across 1920 or
  2560 px, or plugs in a monitor). The chain must survive: the same items stay open, columns are
  added or dropped from the right, and no selection, search text or scroll position is lost.
- **Crossing down into laptop width** with a cascade deeper than the columns available: the deepest
  item stays the one on screen, and growing the window back re-reveals the chain rather than
  resetting it.
- **The auto-opened item disappears** (the Pod is deleted, or a filter no longer matches it). The
  column must say the item is gone rather than showing stale details, and it must not silently jump
  to another item.
- **A very large list** (10,000 objects): choosing the most relevant item must not delay the list
  appearing.
- **The list is still loading** when the section opens. The auto-open must wait for the first page
  and then open, without flashing a wrong item first.
- **Switching context** with a cascade open: the cascade resets to the new context's US1 state, and
  no column keeps the previous cluster's item.
- **A deep link or restored route** that names both a section and an item: it opens as if the
  operator had clicked the section and then that item, not as a prefill of unrelated panes.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: On wide and larger windows the leftmost column MUST be the sections navigator and the
  second column MUST be the selected section, and these two MUST NOT be replaced by a cascade step.
- **FR-002**: The system MUST NOT open any column with content the operator did not ask for. On a
  fresh start at wide or larger, every column after the second MUST be empty until the operator
  drills in — with the single exception in FR-003.
- **FR-003**: At wide width and above, when the operator selects a section, the system MUST open
  that section's most relevant item in the third column. Below wide width nothing is auto-opened. "Most relevant" MUST be the first item in the list's current order whose
  health is not healthy, and the first item in that order when all items are healthy.
- **FR-004**: When a section has no items, or is a destination without a list of items, the third
  column MUST stay empty.
- **FR-005**: Following a link or selecting a child from the item in column N MUST open the target in
  column N+1 while column N keeps showing its own item, for as long as an empty column remains.
- **FR-006**: When no empty column remains, one further drill-in MUST drop the oldest cascade column,
  shift the remaining cascade one column left and open the target in the rightmost column, leaving
  the navigator and the section columns in place.
- **FR-007**: Selecting a different item in the section column MUST replace column 3 and clear every
  column to its right; selecting a different section MUST return the layout to the FR-001/FR-003
  state for that section.
- **FR-008**: The number of columns offered per width class MUST NOT decrease relative to today's
  layout, and MUST NOT decrease as the window grows wider.
- **FR-009**: Phone, tablet and laptop windows MUST behave exactly as they do before this change.
- **FR-010**: A cascade MUST survive a width-class change: items already open stay open, columns are
  gained or lost from the right end only, and each column keeps its selection, its search text and
  its scroll position.
- **FR-011**: Layout profiles saved before this change MUST load without error, and MUST resolve to
  the new behaviour rather than restoring the removed prefilled panes.
- **FR-012**: Empty columns MUST show a short explanation of what the next click will open there.
- **FR-013**: Column widths the operator has set by dragging MUST be preserved when a column fills or
  empties.
- **FR-014**: A column whose item no longer exists MUST say so, and MUST NOT auto-select a different
  item in its place.
- **FR-015**: Changing context MUST reset the cascade to the FR-001/FR-003 state for the new context,
  with no column left showing the previous context's item.
- **FR-016**: The bottom dock MUST be unaffected by this feature: its tabs, height and open state
  behave as they do before this change.

### Key Entities

- **Column (slot)**: one vertical region of the shell. It is either the navigator, the selected
  section, a cascade step, or empty. It has a width the operator can change.
- **Cascade**: the ordered chain of what the operator opened, from the section in column 2 to the
  deepest item. Its length is what decides which columns are occupied.
- **Cascade step**: one link in that chain — a resource, a group of children, or a detail view —
  together with its own selection, search text and scroll position.
- **Relevance order**: the rule that picks the item column 3 opens: first not-healthy item, else
  first item.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a fresh start at wide, ultra and ultra-plus widths, the number of occupied columns
  after choosing one section is exactly three, and zero columns show a section the operator did not
  choose.
- **SC-002**: Reaching a four-level-deep resource (for example cluster → Deployment → ReplicaSet →
  Pod) takes at most three clicks after the section is chosen, with every level of the chain still
  visible on screen at ultra width.
- **SC-003**: Choosing a section shows its list within 100 ms of the data being available, including
  for a list of 10,000 objects, and the auto-opened item does not delay the list appearing.
- **SC-004**: Resizing the window across every width boundary in both directions, with a three-level
  cascade open, loses no open item, no selection, no typed search text and no scroll position — in
  100% of the boundary crossings exercised by the tests.
- **SC-005**: The phone, tablet and laptop end-to-end suites pass unchanged, with no test edits
  needed to accommodate this feature.
- **SC-006**: Every layout profile saved by the previous release loads without error and resolves to
  the new clean state.

## Assumptions

- "Slot" in the feedback means a top-row column of the shell; the bottom dock is a separate surface
  and is out of scope.
- "Small windows" means the phone, tablet and laptop width classes. Desktop (below 1920 px) keeps
  its current three columns and gains the cascade only insofar as its inspector already behaves that
  way; the visible change starts at wide.
- Health for the relevance rule is the health the list already shows for a row; sections whose rows
  have no health notion fall back to the first row.
- The auto-open in column 3 is a navigation the operator can undo by clicking elsewhere; it does not
  need a preference to switch it off in this feature.
- Dropping the Overview and Flux panes from the default wide and ultra profiles does not remove those
  destinations: both stay reachable from the navigator, and Logs, Events and Cluster history stay in
  the dock. Pinning an arbitrary tool into a cascade column by hand is not part of this feature.
- The existing scripted fake backend can express unhealthy rows, empty sections and 10,000-object
  lists, so no new test infrastructure is needed.
