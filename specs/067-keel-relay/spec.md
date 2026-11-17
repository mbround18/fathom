# Feature Specification: Keel relays for hosts Keel cannot be reached from

**Feature Branch**: `067-keel-relay`

**Created**: 2026-11-17

**Status**: Draft

**Input**: User description: "Could Keel offer a keel-proxy that lets nodes check into a remote
endpoint but doesn't serve full Keel itself, just its configuration interfaces? And Keel be aware
of when we have a proxy, or multiple, into different areas? Allowing us to query remote resources
on hub and spoke networks."

Decisions taken with the requester (2026-11-17):

- **Keel dials the relay; runners dial the relay.** Neither ever accepts a connection from the
  other side. The hub needs no inbound port, and runners stay outbound-only (spec 055 FR-031).
- **The relay is untrusted.** It carries the runner's SSH session to Keel byte for byte and never
  terminates it. A relay that is compromised can drop or delay traffic, and nothing more.
- **A relay serves only what a host verifies anyway**: runner binaries pinned by checksum and
  repository content checked by commit signature. Dispatch, approvals and enrollment decisions
  stay on the Keel server.
- **Remote queries are typed**, answered by runners. A relay is never a general network tunnel.

**Builds on**: spec 055 (runners, the runner SSH contract `contracts/runner.md`, signed mode and
host-side verification), spec 058 (the operator's Ansible repository and its trust file
`inventory/group_vars/keel_nodes.yml`), spec 060 (System Settings, enrollment), spec 066
(workspaces, the Hosts page, Add host).

## Why

Keel's server runs on the home LAN (`keel.boop.ninja`, 192.168.1.91). Runners must reach its
runner listener (port 7831) to receive work. Two of the operator's hosts cannot:

- `ns5004891` and `ns534168` are dedicated servers at OVH. From there `keel.boop.ninja` resolves
  to Cloudflare, which does not carry port 7831, and there is no route to the LAN. Checked on
  2026-11-17: the port is closed from `ns5004891`.
- The reverse path works: the home LAN reaches OVH's public addresses directly.

Today the only fixes are outside Keel, and each has a cost:

- **Opening 7831 at home** (a DNS-only record and a port forward) puts an internet-facing listener
  on the home network.
- **Joining the hosts to the tailnet** works, but the tailnet's control plane (Headscale) runs on
  the same k3s cluster Keel is meant to repair. A node that reboots while the cluster is down
  cannot rejoin, and so cannot reach Keel, exactly when Keel is most wanted.
- **Letting Keel SSH into hosts** would make a stolen Keel server root on the whole fleet. Spec
  055 FR-035 exists to prevent that.

The asymmetry is common: a hub that can reach out but should not be reached, and spokes (a
colocation rack, a cloud VPC, a customer site) that can be reached from the hub but cannot reach
it. A relay that sits in the spoke, accepts runners locally and is dialled by the hub fits it
without weakening any trust property Keel already has.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Run jobs on a host Keel cannot be reached from (Priority: P1)

An owner defines a zone, `ovh`, and installs a relay on one host in it. In Keel they register the
relay's address and pin its key. Keel connects out to the relay. The owner enrolls `ns534168`
with its runner pointed at the relay's address. The runner connects to the relay; the relay
carries the session to Keel; the host shows as connected; jobs run on it exactly as on a LAN host,
with signed mode, approvals and host-side verification unchanged.

**Why this priority**: it is the reason for the feature. Without it the OVH hosts cannot go under
Keel at all without one of the trade-offs in "Why".

**Independent Test**: with a Keel server and a relay on separate networks where only the server
can reach the relay, enroll a runner behind the relay, push a signed commit that triggers a job
for it, and confirm the run completes and its log streams in Keel. Then block the server's
outbound connection to the relay and confirm the host shows as disconnected, not failed.

**Acceptance Scenarios**:

1. **Given** a registered relay that Keel can reach, **When** a runner configured with the relay's
   address starts, **Then** it connects through the relay and Keel shows the host as connected
   "via relay `<name>`".
2. **Given** a host connected through a relay, **When** a job for it is dispatched, **Then** the run
   proceeds, streams logs and reports its result exactly as for a directly connected host.
3. **Given** a relay, **When** a runner connects through it, **Then** the runner verifies Keel's
   host certificate against its pinned host CA, for Keel's own runner host name, and refuses the
   connection if it is not Keel answering.
4. **Given** a relay Keel cannot reach, **When** a runner behind it starts, **Then** the runner keeps
   retrying with backoff, the relay holds no state about it, and Keel shows the relay as
   unreachable and its hosts as disconnected.
5. **Given** an enrollment token, **When** the runner enrolls through a relay, **Then** enrollment
   works as it does directly, and the relay never sees the token in clear.

---

### User Story 2 - A relay cannot be used to attack hosts or Keel (Priority: P1)

The relay runs in a network Keel does not control, often on a host that is itself managed. The
owner needs to know that whoever controls the relay host gains nothing beyond the ability to stop
traffic.

**Why this priority**: a relay that could forge jobs, read secrets or reach Keel's web interface
would be a weaker point than the hosts it serves. Story 1 is not shippable without this one.

**Independent Test**: replace the relay with a hostile one (in the test suite, like spec 055's
hostile-server test) that tampers with, replays, reorders and injects bytes, and that tries to
open other channels to Keel. Confirm every attempt fails closed and is audited, and no job runs.

**Acceptance Scenarios**:

1. **Given** a relay that alters bytes of a runner session, **When** the session continues,
   **Then** the SSH integrity check fails and the connection closes. No frame is acted on.
2. **Given** a relay that opens a connection to Keel without a runner behind it, **When** it tries
   to authenticate, **Then** it can only present what any unenrolled client can, and is refused.
3. **Given** a relay, **When** it asks Keel for anything except carrying runner sessions and the
   relay's own health and cache requests, **Then** Keel refuses. The relay link cannot reach the
   web interface, the database or the git forge.
4. **Given** a relay serving cached runner binaries, **When** a served binary does not match the
   checksum pinned in the host's trust file, **Then** the host refuses to install it (as it does
   for binaries from Keel today).
5. **Given** a relay serving repository content, **When** a commit it serves is not the one Keel
   dispatched, or fails signed mode, **Then** the host refuses to run it.
6. **Given** a relay's key, **When** an owner revokes the relay in Keel, **Then** Keel closes the
   link at once and refuses it from then on.

---

### User Story 3 - See zones, relays and how each host is reached (Priority: P1)

The Hosts page groups hosts by zone. Each zone shows its relays with state (connected,
unreachable, revoked), round-trip time, the number of runner sessions carried and the relay's
version. Each host shows whether it is connected directly or through which relay. A Relays
section in Settings registers, edits and revokes relays.

**Why this priority**: the requester asked for Keel to be aware of relays, and an operator cannot
tell "the host is down" from "the relay is down" without it.

**Independent Test**: with two zones, one direct and one behind a relay, confirm the Hosts page
shows both, that stopping the relay turns its zone's hosts to "disconnected (relay unreachable)"
while direct hosts stay connected, and that the change is audited.

**Acceptance Scenarios**:

1. **Given** hosts in two zones, **When** the owner opens Hosts, **Then** hosts are grouped by
   zone and each shows its path ("direct" or "via `<relay>`").
2. **Given** a relay that stops answering, **When** Keel's health check fails, **Then** the relay
   shows as unreachable within one health interval, and its hosts say so instead of showing a
   generic disconnect.
3. **Given** an owner in Settings → Relays, **When** they register a relay, **Then** they enter its
   address and paste its public key, preview and confirm, and the change is audited like any other
   Settings change.

---

### User Story 4 - More than one relay per zone (Priority: P2)

A zone may have two relays on different hosts. Runners in the zone list both. If one relay goes
away, runners reconnect through the other and runs continue to dispatch.

**Why this priority**: a single relay is a single point of failure for the zone. It is not needed
for the first two hosts, but it is where relays become trustworthy for anything important.

**Independent Test**: with two relays in one zone and a runner configured with both, stop the
relay it is using and confirm the runner reconnects through the other within its retry interval,
and that a run queued meanwhile dispatches once it does.

**Acceptance Scenarios**:

1. **Given** a runner with two relay addresses, **When** the one in use stops, **Then** the runner
   connects through the other and Keel shows the new path.
2. **Given** a run in progress through a relay that stops, **When** the session drops, **Then** the
   run is handled as a lost runner connection is today (spec 055), and is not run twice.

---

### User Story 5 - Ask hosts behind a relay typed questions (Priority: P3)

From a host's page, an owner can ask questions the runner answers: facts it already gathers, the
applied revision, disk and filesystem health, the state of named systemd units, free space under
named paths. The answers travel over the runner's existing session, so they work the same direct
or through a relay.

**Why this priority**: the requester wants to "query remote resources on hub and spoke networks".
Typed questions give that without turning a relay into a way into the spoke's network.

**Independent Test**: ask a host behind a relay for the state of a named unit and its disk
health, and confirm the answer. Ask for a unit or path outside the host's allowed query list and
confirm the runner refuses.

**Acceptance Scenarios**:

1. **Given** a connected host, **When** an owner asks a query from the allowed set, **Then** the
   runner answers within the query timeout and the answer is shown with its time.
2. **Given** a query the host's trust file does not allow, **When** it arrives, **Then** the runner
   refuses it, and the refusal is audited on both sides.
3. **Given** any query, **When** it is answered, **Then** the runner never runs a shell command
   built from the query's parameters.

---

### Edge Cases

- A relay host is itself under Keel, and a job on it restarts the relay: runners behind it
  reconnect (through a second relay if the zone has one). The relay's own job must not be blocked
  waiting on the relay. Keel MUST warn when a job's target hosts include the only relay of a zone
  that other targets depend on.
- The relay's host is behind the same failure Keel is meant to fix (for example the relay runs in
  a Kubernetes cluster that is down). Relays are documented as host services, not cluster
  workloads, for this reason.
- Two Keel servers try to dial the same relay: the relay accepts only the Keel identity it was
  configured with.
- A relay restarts mid-run: as Story 4 scenario 2.
- A runner configured with a relay address is moved to the LAN: it may list the direct address
  too, and uses the first that answers. The path Keel shows follows.
- Clock skew on the relay host: the relay checks no certificate validity on runner sessions (it
  cannot see them), so skew affects only the Keel↔relay link.
- A relay serves a stale cache of the repository: the host fetches the commit Keel dispatched by
  hash. If the relay does not have it, the relay fetches it over the link. It never substitutes
  another commit.
- A zone with no relays and no direct reachability: hosts in it show as "no path", not as failed.

## Requirements *(mandatory)*

### Functional Requirements

**Relay link (Stories 1, 2)**

- **FR-001**: Keel MUST connect out to each registered relay. A relay MUST NOT accept a link from
  any Keel identity except the one pinned in its configuration, and Keel MUST NOT accept a relay
  that does not present the key registered for it.
- **FR-002**: A relay MUST accept runner connections on a configured local address and carry each
  one to Keel as an opaque byte stream over the link. The relay MUST NOT terminate, decrypt or
  inspect the runner's SSH session.
- **FR-003**: The runner contract (spec 055 `contracts/runner.md`) MUST hold unchanged end to end:
  same algorithms, same server authentication against the pinned host CA, same subsystems, same
  refusals. The runner MUST verify Keel's host certificate for Keel's runner host name, not the
  relay's address.
- **FR-004**: The relay link MUST carry only runner byte streams, relay health and the cache
  requests in FR-010. Keel MUST refuse anything else on it.
- **FR-005**: Keel MUST record, per runner session, the relay it arrived through, and expose it to
  the Hosts page and the audit log.
- **FR-006**: Revoking a relay MUST close its link at once and refuse it afterwards. Hosts behind
  it show as disconnected with the reason.

**Runner (Stories 1, 4)**

- **FR-007**: The runner MUST accept an ordered list of addresses to dial, separately from the
  host name it verifies Keel's certificate for. It dials them in order with backoff and uses the
  first that completes the SSH handshake with Keel.
- **FR-008**: The operator's trust file MUST be able to set these addresses per host or per group
  (`keel_runner_server` stays the verified name; a new `keel_runner_dial` list holds addresses).
  The Ansible role MUST refuse a dial list without a verified name.

**Zones and visibility (Story 3)**

- **FR-009**: Keel MUST have zones within a workspace. A host belongs to one zone. A relay serves
  one zone. Hosts without a zone are in the implicit `direct` zone.
- **FR-010**: A relay MAY cache runner release files and git objects for its zone, fetched over
  the link. Hosts MUST verify cached content exactly as content from Keel: release files against
  the checksums in the host's trust file, commits by hash and by signed mode.
- **FR-011**: Keel MUST health-check each relay at a configured interval and show state, round-trip
  time, carried sessions and version. Registering, editing and revoking relays MUST go through the
  Settings preview and confirm flow and be audited.

**Typed queries (Story 5)**

- **FR-012**: Keel MUST offer a fixed set of query types answered by the runner over its existing
  session: facts, applied revision, unit state, disk health, path free space.
- **FR-013**: Each host's trust file MUST list which query types, units and paths it answers. The
  runner MUST refuse anything else and MUST NOT build a shell command from query parameters.
- **FR-014**: Queries MUST be audited on Keel and on the host, with who asked.

**Packaging**

- **FR-015**: The relay MUST ship as its own binary (`keel-relay`) with a systemd unit, from the
  same release as the runner, pinned by checksum in the trust file like the runner's binaries.
  The operator's Ansible repository gets a `keel_relay` role and a `keel-relay` profile.

### Key Entities

- **Zone**: a named group of hosts in a workspace that share network reachability. Has relays.
- **Relay**: a registered `keel-relay` instance. Name, zone, address Keel dials, public key,
  state, last health, version, revoked flag.
- **Relay link**: Keel's outbound connection to one relay, carrying runner streams.
- **Runner path**: per runner session, `direct` or the relay it came through.
- **Query**: a typed request to a runner, its parameters, who asked, the answer or refusal, and
  times.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `ns534168` and `ns5004891` run Keel jobs through a relay, with no inbound port open
  on the home network and no dependency on the tailnet.
- **SC-002**: The hostile-relay test suite shows every tampering, replay, injection and
  side-channel attempt in Story 2 failing closed, with no job run and an audit entry for each.
- **SC-003**: When a relay stops, its hosts show "relay unreachable" within one health interval
  (default 30 s), and with a second relay in the zone, runners reconnect within their retry
  interval (default 60 s).
- **SC-004**: A run through a relay adds no more than the link's round-trip time to frame delivery;
  log streaming through a relay keeps up with a run that writes 1 MiB of output a minute.
- **SC-005**: An owner can tell from the Hosts page alone whether a disconnected host's relay or
  the host itself is down.

## Assumptions

- The hub (Keel's server) can open TCP connections to each relay's address. Where it cannot,
  this feature does not help, and a zone falls back to the other options in "Why".
- One relay host per zone is enough to start. Story 4 adds the second.
- Relays run on Linux hosts with systemd, like runners.
- Runner and relay ship together with the server. A runner too old to take a dial list keeps
  working direct.
- The relay's own host may be under Keel. It is not required to be.

## Open Questions

Both resolved with the owner on 2026-11-17 (research.md R1, R2):

- **Link transport**: SSH over russh. Keel is the client and the relay the server; each runner
  stream is a `forwarded-tcpip` channel the relay opens to Keel; health and cache share the
  `keel-relay-v1` subsystem.
- **Relay registration**: paste keys both ways. The relay's generated Ed25519 key is pasted into
  Settings → Relays; Keel's relay-link key goes into the trust file as `keel_relay_hub_keys`.

Narrowed at plan time (research.md R7): FR-010's git-object caching is not built, because
repository content travels inside the runner session as `Pack` frames, where the relay cannot see
it. Release-file caching stays, and is blocked on spec 055's `/runner` route.

## Out of Scope

- Relays that dial the hub (for spokes the hub cannot reach but that can reach the hub). Possible
  later, with the same "untrusted relay" rule.
- Chaining relays through each other.
- Generic port forwarding, SOCKS or any tunnel into a spoke network.
- Serving Keel's web interface or API through a relay.
- Running playbooks on the relay against other hosts (the executor's forced `--limit` and local
  connection stay as they are).
- Fixing the readiness gaps found on 2026-11-17 (runner distribution route, per-host run lock,
  join tokens). They block this feature's testing on real hosts, and belong to specs 055 and 058.
