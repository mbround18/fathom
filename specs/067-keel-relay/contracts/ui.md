# UI contract: zones, relays, host paths and queries

`apps/keel/web`. Mobile-first (Principle IV); every element below is reachable by keyboard and
touch. New pages and dialogs are lazy-loaded (`React.lazy`), like the other feature routes.

## Hosts page (`features/hosts/HostsPage.tsx`)

- Hosts are grouped by zone (`direct` first, then by name). Each group header shows the zone's
  relays as chips: name, state (connected / unreachable / quarantined / revoked, with an icon as well as
  colour), RTT and stream count at ≥ 768 px; name and state only below.
- Each host row gains a **Path** column: `direct`, `via <relay>`, or the reach text from
  data-model.md (`disconnected (relay unreachable)`, `no path`, …). Column priority: below
  state, above versions.
- Host inspector: zone (with a "Change zone" action → preview/confirm), last path and its
  advisory peer, and a **Queries** panel (Story 5): pick a kind, fill the one parameter if any,
  Ask. Answers list newest first with time, asker and outcome. A host without `query-v1` shows
  "runner too old for queries" instead of the form.

## Settings → Relays (new tab, `features/settings/RelaysTab.tsx`)

- **Keel's relay-link key**: the active public line with a copy button, labelled "Add this to
  `keel_relay_hub_keys`". Pending/retired keys and rotate/activate/retire actions for instance
  admins.
- **Zones** list with create, rename, remove (preview lists blockers).
- **Relays** list: name, zone, dial address, fingerprint, state, version. Register / Edit open a
  dialog: name, zone, dial address, pasted public key, optional "runs on host". Preview shows the
  parsed fingerprint before confirm. Revoke is a separate destructive action naming the relay and
  the hosts currently connected through it (Principle I).

## Run preview

When targets include the only non-revoked relay of a zone and other targets in that zone, the
preview shows a warning naming the relay host and the dependent hosts (edge case, research R10).

## E2E

Playwright project `s067` (`apps/keel/web/tests/e2e/s067/`) against the fake backend
(`fake-relays.ts`): grouping and paths, relay unreachable turning only its zone's hosts, register
with preview, revoke, query answered / refused / too old, and the run preview warning. Plus
`polish-screens` shots of the Hosts page with zones and the Relays tab at 360 and 1280 px.
