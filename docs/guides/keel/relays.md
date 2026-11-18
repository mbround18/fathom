# Relays

A relay lets Keel manage hosts that cannot reach Keel. You install it on one host in the remote
network, Keel connects out to it, and the hosts there connect to the relay instead of to Keel.
This guide covers when to use one, zones, registering a relay, and what a relay can and cannot do.

## When to use a relay

Hosts normally connect straight to Keel's runner port (7831). Use a relay when they can't, but
Keel can reach them. For example:

- Keel runs at home, and some hosts are rented servers or a cloud network with no route back to
  your home network;
- you don't want to open a port at home, or to depend on a VPN that may be down when you need
  Keel most.

The relay sits in the remote network. Keel dials it; the remote hosts dial it. Neither side ever
accepts a connection from the other, so Keel still needs no inbound port.

You don't need a relay when the hosts can already reach Keel's runner port. A relay doesn't help
when Keel can't reach the relay either: some address in the remote network has to accept
connections from Keel.

## Zones

A zone is a group of hosts that share a network, such as one data centre or one cloud network.
Each relay serves one zone, and each host belongs to at most one zone. Hosts without a zone are in
`direct`, and connect to Keel straight.

On **Hosts**, hosts are grouped by zone once the workspace has one, with `direct` first. Each
zone's header shows its relays with their state, and each host shows its **Path**:

| Path | What it means |
|---|---|
| `direct` | Connected straight to Keel |
| `via <relay>` | Connected through that relay |
| `disconnected` | The host isn't connected |
| `disconnected (relay unreachable)` | Keel can't reach the zone's relays, so the host can't connect |
| `disconnected (relay revoked)` | The host's last session came through a relay that has since been revoked |
| `no path` | The host is in a zone that has no relay |

So when a host goes quiet, the Path tells you whether the host or its relay is down.

### Create a zone

Creating, renaming and removing zones needs the Workspace admin role.

1. Open **Settings → Relays** in the workspace.
2. Under **Zones**, choose **Create zone**.
3. Enter a **Name** (lowercase letters, digits and dashes, not `direct`) and, if you like, a
   **Description**.
4. Preview, check it, and confirm.

To put a host in a zone, open the host on **Hosts** and choose **Change zone** in its inspector.
The preview warns you when the zone has no relay yet. Removing a zone is refused while hosts or
relays are still in it; the preview lists them.

## Register a relay

Registering a relay pins keys both ways. You paste the relay's key into Keel, so Keel only talks
to that relay, and you put Keel's key on the relay, so the relay only accepts Keel.

You need the Workspace admin role, and a zone for the relay.

### Install the relay

Install `keel-relay` on a host in the zone as a host service, with the `keel-relay` profile from
your Ansible repository. A host can run both a runner and a relay. The `keel_relay` role:

1. installs `keel-relay`, checking it against the checksum pinned in `keel_relay_sha256`, from
   the same release as the runner;
2. writes `/etc/keel-relay/relay.toml` and the `hub_keys` file;
3. starts `keel-relay.service`;
4. prints the relay's key: its public line, then its `SHA256:` fingerprint.

The role's main variables:

| Variable | What it sets |
|---|---|
| `keel_relay_hub_keys` | Keel's relay-link key lines, from **Settings → Relays**. Required |
| `keel_relay_link_listen` | The address Keel dials, `0.0.0.0:7832` by default |
| `keel_relay_runner_listen` | The address the zone's runners dial, `0.0.0.0:7831` by default |
| `keel_relay_max_streams` | The most runner connections the relay carries at once, 256 by default |

On the host itself, `keel-relay key` prints the same two lines at any time.

Open the relay's link port (7832) only to Keel's address, and its runner port (7831) only to the
zone's hosts. The role doesn't change your firewall.

### Paste the keys

1. Open **Settings → Relays**. Under **Relay-link key**, copy Keel's key line. Only an instance
   admin sees it here; a workspace admin sees the same line in the Register preview.
2. Add it to `keel_relay_hub_keys` and run the `keel-relay` profile again, so the relay accepts
   Keel.
3. Under **Relays**, choose **Register relay** and fill in:
   - **Name**: lowercase letters, digits and dashes;
   - **Zone**;
   - **Dial address**: where Keel reaches the relay, as `host:port`;
   - **Host key**: the relay's whole `ssh-ed25519` line, as `keel-relay key` printed it;
   - **Runs on host** (optional): the enrolled host the relay runs on, so Keel can warn you
     before a run restarts it.
4. Preview. The preview shows the key's fingerprint: check it against the `SHA256:` line the
   relay printed. Confirm with your passkey.

Keel connects to the relay within one health check (30 seconds by default). The relay then shows
**connected**, with its round-trip time, the number of runner connections it carries, and its
version.

### Point the zone's hosts at the relay

Set `keel_runner_dial` for the zone's hosts in your Ansible repository, and run the base profile
on them again:

```yaml
keel_runner_server: keel.example.net:7831   # the name Keel's certificate is checked for
keel_runner_dial:
  - relay-1.example.net:7831                # the relay's runner port
```

`keel_runner_server` stays Keel's own name: the runner checks Keel's certificate against that
name whatever address it dials. `keel_runner_dial` is the list of addresses to try, in order. List
two relays for a zone with two, and add Keel's direct address last if a host can sometimes reach
Keel straight. The host's Path follows whichever address answered.

A new host enrolls through the relay the same way it would directly, with its enrollment token.

### Rotate Keel's relay-link key

An instance admin can choose **Rotate** under **Relay-link key**. Keel makes a pending key and
keeps using the active one. Add the pending line to every relay's `keel_relay_hub_keys`, then
choose **Activate**. Remove the old line from the relays afterwards. **Retire** drops a pending key
you no longer want.

## What a relay can and cannot do

Treat the relay's host as untrusted. Keel does.

### It can only drop or delay traffic

A runner's connection to Keel is encrypted end to end. The relay passes the bytes along without
being able to read or change them:

- the runner checks Keel's certificate for Keel's own name, so a relay that pretends to be Keel
  gets nothing, not even an enrollment token;
- a changed, replayed, reordered or injected byte breaks the connection, and nothing in it is
  acted on;
- the relay can't reach Keel's web interface, API, database or Git repositories, and it can't
  open a shell or a tunnel to anything.

The worst a relay can do is stop or slow the zone's connections. Hosts then show
`disconnected (relay unreachable)`, and keep retrying.

### It serves only its own workspace

A relay belongs to one workspace. Keel refuses any host, enrollment or join that a relay carries
for a different workspace, and records the refusal. The relay may carry any host of its own
workspace, whatever the host's zone.

### Quarantine and release

Some things an honest relay never does: present a host key other than the one you registered,
open a kind of connection it isn't allowed, or send a message Keel doesn't expect. If a relay does
any of them, Keel **quarantines** it: it drops the relay's connection and every host session it
carries, records why, and stops dialling it.

A quarantined relay shows its reason in **Settings → Relays**. Find out what happened on the
relay's host first. Then choose **Release**: the preview shows the reason, when it happened, and
the refusals since. Confirm with your passkey, and Keel dials the relay again.

A relay that is merely busy isn't quarantined. Keel limits how many connections a relay carries
and how fast it opens them, and refuses the excess without taking the relay out of service.

### Revoke a relay

Choose **Revoke** on the relay's row. The preview names the hosts connected through it. Once you
confirm, Keel closes the relay's connection and every session through it at once, and never dials
it again. Its hosts show `disconnected (relay revoked)` until they reach Keel another way.
Revoking can't be undone, and Keel won't accept a revoked relay's key again. To use the same host
as a relay again, delete `/var/lib/keel-relay/host_ed25519` there, run `keel-relay key` for a new
key, and register it as a new relay.

## Relays are host services

Install relays on hosts with systemd, never as workloads in a Kubernetes cluster. A relay is
there for the moments when something is broken, and Keel is how you fix it. If the relay ran in a
cluster that is down, the hosts behind it couldn't reach Keel exactly when you need them to.

For the same reason, if a zone has a single relay and a run targets both the relay's host and
other hosts in the zone, the run's preview warns you: restarting that host cuts the others off
until it is back. A second relay in the zone, listed in `keel_runner_dial`, avoids this.

## Queries

Coming soon: asking hosts typed questions, such as a unit's state or a disk's health, the same way
whether they are connected directly or through a relay. Each host will answer only the questions
its trust files allow.
