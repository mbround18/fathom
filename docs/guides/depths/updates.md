# Image updates

Depths can tell you which services run an image older than the one their tag points to now, on
every stack at once, so you don't have to press Pull on each stack to find out.

Checking only reads: Depths asks each image's registry what the tag points to and compares that
with what the container runs. A check never pulls an image or restarts a container.

## Who sees what

- **Admins** get the **Updates** page in the top bar. The number next to it is how many stacks
  have at least one service with an update.
- **Users** see the update state of each service on the pages of the stacks assigned to them.
  They don't get the Updates page or the count.

The whole feature can be removed with `DEPTHS_UPDATES=off` (see
[All settings](install.md#all-settings)). Then there is no Updates page and Depths never talks to
a registry.

## Turning checks on

Checks start **off**. Until an admin turns them on, Depths asks no registry anything, and the
Updates page says so.

1. Open **Updates**.
2. Pick how often to check (every hour, 6 hours, 12 hours or day).
3. Choose **Turn checks on**, read the summary of the change, and choose **Confirm**.

The change is recorded in the audit log. From then on Depths checks on that schedule; the header
shows the interval, when the next check is due, and the host's time zone. You can change the
interval or turn checks off from the same page.

**Check now** checks every image straight away. If a check is already running, Depths says how
long to wait before asking again.

Changing the settings and **Check now** need a trusted network. From an untrusted one you can
still read the page (see [Networks and the restricted view](security.md)).

## What each state means

| State | Meaning |
|---|---|
| Update available | The tag now points to a different image than the one running. The page shows the running and the new version (or digest) |
| Up to date | The container runs what the tag points to |
| Not checked | Not checked yet, or no container is running for the service |
| Can't check | The registry could not be asked. The reason names it: no credentials, a credential helper, a rate limit, or the registry being unreachable |
| Excluded | Depths never checks this service. The reason says why: pinned by digest, built locally, a local image, a Watchtower label, or Depths' own container |

States for later features (Held, Rejected, Rolled back and others) appear once automatic updates
arrive.

Filter the list by state or by stack. Every row links to its stack.

## Depths' own container

Depths never updates itself, because recreating its own container would cut the update off. Its
row shows whether a newer image exists, and the commands to run by hand in the directory that
holds its compose file:

```sh
docker compose pull depths
docker compose up -d depths
```

## Private registries

Images on a private registry need a `config.json` with credentials mounted into Depths. Until
then, those services show **Can't check**. The steps are in
[Installing Depths](install.md#private-registries).

At the bottom of the Updates page, **Registries** lists every registry Depths has talked to, how
it authenticates to each (anonymously, or with credentials from `config.json`), and whether a
registry is being given a rest after a rate limit or an error. Depths never shows or stores the
credentials themselves.
