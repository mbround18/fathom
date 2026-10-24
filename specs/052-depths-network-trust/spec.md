# Feature Specification: Depths network trust and sessions

**Feature Branch**: `050-depths-stack-manager` (spec 052 builds on 050 and 051 and lands on the same branch)

**Created**: 2026-10-24

**Status**: Draft

**Input**: User description: "We might be able to track where the user logs in from line ips and prevent dangerous things if its not on an ip we havent seen or know about for at least a week (configurable) and let the uwer see their sessions and ips and locations" — then: "put it in the users control too to white list an ip address (this would be a restricted feature) we should implicit trust the first ip they log in with but anytbjng else is unknown. But we shojld ask them is the ip they are access feom their home network or some where public. If they say public we give them the restricted view in csse they need to stop or restart a server for maintenance but thats about it"

## Clarifications

### Session 2026-10-24

- Q: Which network is trusted without anyone vouching for it? → A: The first network a person ever signs in from. Every other network starts out unknown.
- Q: What does a person on an unknown network get? → A: Depths asks whether the network is their home or somewhere public. Either way they get the restricted view, which is enough to see stacks and stop, start or restart them for maintenance.
- Q: Does answering "home" make the network trusted immediately? → A: No. A stolen session could give that answer too. "Home" starts the network towards trust: it becomes trusted once it has been used for the maturity window (7 days by default) on at least two separate days, or straight away if an administrator whitelists it. "Public" networks never become trusted on their own.
- Q: Who can whitelist a network? → A: Administrators, from the users page, and only while they are themselves on a trusted network. Whitelisting is audited. Nobody can whitelist their own current untrusted network.
- Q: Where does location come from? → A: Only from an optional geo-IP database file the operator provides. Depths never sends addresses to an outside service. Location is shown, never used to decide trust.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Dangerous actions only from trusted networks (Priority: P1)

A person signs in from home for the first time; that network is trusted and everything their role
allows works. Later they sign in from a hotel. Depths sees a network it does not know, asks "Is this
your home network or somewhere public?", and whatever they answer they get the restricted view: they can
see their stacks, read logs and stats, and start, stop or restart stacks, but shells, edits, revealing
secrets, deleting, pulling images, user management and whitelisting are refused, with a message saying
why and when this network could become trusted. The server enforces this on every request; the UI
mirrors it so the person is not shown controls that will be refused.

**Why this priority**: A stolen session cookie or a compromised account used from elsewhere is the most
likely way an outsider reaches Depths. Limiting what an unknown network can do contains that, while still
letting the owner deal with an outage from their phone.

**Independent Test**: Sign in from network A (first ever) and confirm a shell opens. Sign in from network
B, answer "public", and confirm the stack list, logs and a restart work while shell, edit, reveal, delete,
pull and the users page are refused with the untrusted-network reason and each refusal is audited.

**Acceptance Scenarios**:

1. **Given** a person with no sign-in history, **When** they sign in, **Then** that network is recorded as trusted (first network) and they are not asked anything.
2. **Given** a person signed in from a network that is not trusted, **When** they first load Depths from it, **Then** they are asked whether it is home or public, and the restricted view applies whatever they answer and until they answer.
3. **Given** the restricted view, **When** the person views stacks, files with secrets masked, logs or stats, or starts, stops or restarts a stack, **Then** it works.
4. **Given** the restricted view, **When** the person opens a shell, saves a file, reveals a secret, takes a stack down, deletes a stack, pulls images, removes volumes or images, changes users, whitelists a network or exports the audit log, **Then** it is refused with the untrusted-network reason, the refusal is audited, and nothing changes.
5. **Given** a crafted request sent directly to the server from an untrusted network, **When** it asks for a restricted action, **Then** it is refused the same way: hiding the control is not the only protection.
6. **Given** a person who answered "home" on a network, **When** they have used it on at least two separate days and the first use is at least the maturity window ago, **Then** the network becomes trusted and the full view returns without signing in again.
7. **Given** a person who answered "public", **When** any amount of time passes, **Then** the network stays restricted unless an administrator whitelists it or the person changes their answer to "home" (which starts the maturity clock then).
8. **Given** a network inside the operator's always-trusted list (for example the LAN or the tailnet), **When** anyone signs in from it, **Then** it is trusted without asking.

---

### User Story 2 - See and end your own sessions (Priority: P1)

A person opens "Your sessions" from the account menu. They see each signed-in session: when it started,
when it was last used, its address, whether that network is trusted, an approximate location when
available, and the browser and device. Their current session is marked. They can end any other session,
or all of them at once, from any network, including an untrusted one, so that someone who notices a
session they do not recognise can always throw it out. They also see the networks Depths knows for
them, with first and last use, their answer (home or public), trust state and location.

**Why this priority**: Being able to see and kill a hijacked session is the companion to Story 1, and it
is the one safety action that must never be blocked.

**Independent Test**: Sign in from two browsers. In the first, open Your sessions, confirm both are listed
with correct details and the current one is marked, end the second, and confirm the second browser is
signed out immediately (its open pages and streams close) and the ending is audited.

**Acceptance Scenarios**:

1. **Given** a person with three sessions, **When** they open Your sessions, **Then** all three are listed with start, last use, address, trust state, location (or "unknown") and browser, and the current one is marked.
2. **Given** an untrusted network, **When** the person ends another session or all other sessions, **Then** it works, the ended sessions can no longer make any request, their live connections close within 5 seconds, and it is audited.
3. **Given** the known-networks list, **When** the person changes a network's answer between home and public, **Then** the change is recorded and audited, and a change to "home" never makes the network trusted immediately.
4. **Given** a browser or device string supplied by the client, **When** it is shown, **Then** it is shown as plain text, shortened, and cannot inject markup.

---

### User Story 3 - Administrators whitelist and review networks (Priority: P2)

An administrator opens a person on the users page and sees that person's sessions and known networks.
They can whitelist an address or address block for that person (with an optional label), remove a
whitelist entry, forget a known network, and end the person's sessions. Whitelisting and forgetting are
restricted actions: they only work while the administrator is on a trusted network, and they are
audited. The operator can also list always-trusted networks in Depths' settings, which is how an owner
who has lost every trusted network recovers.

**Why this priority**: It covers the cases maturity does not: a new home connection, a colleague who
needs full access today, and removing trust from a network that should not have it.

**Independent Test**: As an administrator on a trusted network, whitelist network C for user U; from C,
confirm U has the full view. Remove it and confirm U's next request from C is restricted. From an
untrusted network, confirm the administrator's whitelist attempt is refused and audited.

**Acceptance Scenarios**:

1. **Given** an administrator on a trusted network, **When** they whitelist an address or block for a person, **Then** requests from that person on that network are trusted from the next request, and the change is audited with who, for whom, the network and the label.
2. **Given** an administrator on an untrusted network, **When** they try to whitelist, remove a whitelist entry or forget a network, **Then** it is refused and audited.
3. **Given** a person, **When** an administrator (or the person) tries to whitelist the network the requester is currently on while it is untrusted for the requester, **Then** it is refused.
4. **Given** a whitelist entry is removed or a network forgotten, **When** the person's next request arrives from it, **Then** it is treated as unknown (asked home or public, restricted).
5. **Given** an administrator, **When** they end another person's sessions, **Then** it works from any network and is audited.
6. **Given** the owner's only trusted network has changed (a new home address), **When** the operator adds the new network to the always-trusted setting and restarts Depths, **Then** the owner has the full view from it.

---

### User Story 4 - The client address is right, or Depths says it is not (Priority: P2)

Network trust only means something if Depths sees each visitor's real address. When Depths runs behind a
reverse proxy or tunnel, the operator lists it as trusted so Depths reads the forwarded client address.
If the network gate is on and every visitor appears to come from a private or proxy address, Depths warns
in its log and shows administrators a notice, because everyone would then look like one network.

**Why this priority**: A misconfigured proxy silently turns the feature off (everyone shares the
"first network") or on for everyone; operators need to be told.

**Independent Test**: Run with the gate on and no trusted proxies while requests arrive through a local
proxy; confirm the start-up or first-requests warning and the administrator notice. Add the proxy to
the trusted list and confirm sessions now show the forwarded addresses and the notice is gone.

**Acceptance Scenarios**:

1. **Given** the gate on and no trusted proxies configured, **When** the first sign-ins all come from private addresses, **Then** Depths logs a warning naming the trusted-proxies setting and administrators see a notice.
2. **Given** a trusted proxy that forwards the client address, **When** a person signs in through it, **Then** their session and network show the forwarded address, not the proxy's.

### Edge Cases

- A person's home address changes (dynamic addresses): the new address is unknown. They get the restricted view; it matures after the window if they answer "home", or an administrator whitelists it sooner. The address-grouping setting lets an operator group addresses by a wider block.
- IPv6 privacy addresses change often: IPv6 addresses are grouped by their /64 by default, so one home connection is one network.
- An administrator is the only administrator and is on an untrusted network: they still have the restricted view and can end sessions; recovery is maturity or the always-trusted setting.
- The network changes in the middle of a session (a phone moving from Wi-Fi to mobile data): each request is judged by the network it comes from; an open shell started on a trusted network ends when a request from the same session arrives from an untrusted one.
- Two people share a network: trust is per person; one person's history does not make the network trusted for the other.
- The first network for existing users when this feature is turned on: the first network each person signs in from after the upgrade becomes their first network, and this is audited, so an administrator can review it.
- An unparseable or missing geo-IP file: location shows "unknown" and Depths says why in its log at start-up; nothing else is affected.
- Private, loopback and tailnet addresses show "private network" as their location.
- A person with hundreds of known networks: the list is bounded; the least recently used are dropped first, and networks unused for 180 days are forgotten (except whitelist entries, which stay until removed).
- The gate is turned off: every network is trusted, the question is never asked, and sessions and networks are still recorded and shown.

## Requirements *(mandatory)*

### Functional Requirements

**Recording networks**

- **FR-001**: Depths MUST record, for each person, each network they sign in or make requests from: first use, last use, the distinct days used, how it became known (first network, answer, whitelist, always-trusted) and their home/public answer. A network is an IPv4 address or an IPv6 /64 by default; the grouping MUST be configurable separately for IPv4 and IPv6.
- **FR-002**: Recording MUST happen at sign-in and, for authenticated requests, no more than once per hour per session.
- **FR-003**: Each person's network list MUST be bounded; when full, the least recently used non-whitelisted network is dropped, and non-whitelisted networks unused for 180 days are forgotten.

**Trust rule**

- **FR-004**: A network MUST be trusted for a person when any of these hold: it is inside the operator's always-trusted list; it is the person's first network; an administrator whitelisted it for the person; or the person answered "home" for it, its first use is at least the maturity window ago (7 days by default, configurable; 0 means networks never mature on their own), and it was used on at least two separate days.
- **FR-005**: A network the person answered "public" for MUST NOT become trusted by maturity.
- **FR-006**: A person's own answer MUST NOT make a network trusted immediately, and nobody may whitelist a network from a request that arrives on a network untrusted for the requester.
- **FR-007**: Trust MUST be per person.
- **FR-008**: The gate MUST be on by default and MUST be possible to turn off with a setting; turned off, every network is trusted.

**Restricted view**

- **FR-009**: When the network a request comes from is not trusted for its sender, only these MUST be allowed (subject to the person's role from spec 050): viewing stacks, containers, files with secrets masked, logs, stats and the person's own sessions and networks; starting, stopping and restarting stacks and containers; answering the home/public question; ending sessions.
- **FR-010**: Every other action MUST be refused with an untrusted-network reason that says when, if ever, the network would become trusted, including at least: shells, saving files, revealing secrets, bringing a stack down, deleting stacks, pulling images, removing volumes or images, changes to users and roles, whitelisting and forgetting networks, the compose "deploy anyway" acknowledgement (spec 051) and exporting the audit log.
- **FR-011**: The check MUST be made server-side on every request and command, after the role check, for page actions and live connections alike. Hiding controls in the UI MUST NOT be the only protection.
- **FR-012**: An open shell MUST end when a later request in the same session arrives from a network that is untrusted for the person.
- **FR-013**: The UI MUST show a clear restricted-view banner on an untrusted network, ask the home/public question once per network until answered, and mark refused controls with the reason.

**Sessions**

- **FR-014**: Every person MUST be able to list their active sessions with start, last use, address, network trust state, approximate location and a shortened browser/device description, with their current session marked.
- **FR-015**: Every person MUST be able to end any of their other sessions, or all of them, from any network. Ending a session MUST stop it from making any further request and close its live connections within 5 seconds.
- **FR-016**: Every person MUST be able to list their known networks with first and last use, answer, how it became known, trust state and location, and change a network's answer.
- **FR-017**: Administrators MUST be able to see any person's sessions and networks, end their sessions from any network, and, from a trusted network only, whitelist an address or block for them with an optional label, remove whitelist entries and forget networks.

**Location and addresses**

- **FR-018**: Location MUST come only from an optional offline geo-IP database configured by the operator. Depths MUST NOT send addresses to any outside service. Without a database, or for addresses not found, location MUST show "unknown"; private, loopback and tailnet addresses MUST show "private network".
- **FR-019**: Location MUST NOT be used in any trust decision.
- **FR-020**: The client address MUST be taken from forwarding headers only when they come from a configured trusted proxy (as in spec 050).
- **FR-021**: With the gate on, when no trusted proxies are configured and the first sign-ins all come from private addresses, Depths MUST log a warning naming the setting and show administrators a notice.

**Audit**

- **FR-022**: Depths MUST audit: a person's first network being set; each new network seen; each home/public answer and change; each whitelist added or removed; each network forgotten; each session ended (by whom); and every refusal for an untrusted network. Entries record addresses and networks but never location text or secrets.

**Docs**

- **FR-023**: The user guides MUST explain the trust rule, the restricted view, the question, whitelisting, sessions, the settings (gate, maturity window, address grouping, always-trusted networks, geo-IP file), the proxy requirement, and how to recover when every trusted network is lost.

### Key Entities

- **Known network**: per person; the network (address or block), first and last use, distinct days used, source (first, answer, whitelist, always-trusted), answer (home, public, none), whitelist label and who whitelisted it.
- **Session record**: an active signed-in session; start, last use, last address, browser/device description; ending it revokes it.
- **Trust decision**: for a request, trusted or restricted, with the reason and, if restricted, when it would mature.
- **Settings**: gate on/off, maturity window in days, IPv4 and IPv6 grouping, always-trusted networks, geo-IP database location.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From an untrusted network, 100% of the actions listed in FR-010 are refused in automated tests, whether sent through the UI or directly.
- **SC-002**: From an untrusted network, a person can restart a stack in under 30 seconds from opening Depths, including answering the question.
- **SC-003**: An ended session is unable to make any request immediately and its live connections close within 5 seconds, in 100% of tested cases.
- **SC-004**: A network answered "home" becomes trusted no earlier than the maturity window and only after use on two separate days, in 100% of tested clock scenarios.
- **SC-005**: No address is sent to any outside service in any configuration (verified by test with no network access).
- **SC-006**: A person can find and end an unrecognised session in under 1 minute from any page.

## Assumptions

- Roles, sign-in, sessions, the audit log and the trusted-proxy setting come from spec 050. The owner counts as an administrator and is subject to the gate like everyone else.
- The network gate composes with spec 051's shell guard: a shell needs both to allow it.
- Session cookies cannot be tied to a device by Depths alone; a thief who copies a cookie gets whatever the network they use it from allows, which is the point of the gate.
- Notifying people about sign-ins from new networks (email, chat) is out of scope for this spec.
- The recommended geo-IP database is the free DB-IP "IP to City Lite" file, whose licence requires attribution; the guides say how to download and attribute it.
