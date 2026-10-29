# Keeping Keel up to date

Keel can tell you which build it runs, and whether a newer image has been published. An owner
can then update it from the browser, through a reviewed run. Keel never updates itself without an
owner's confirmation.

## Which build is running

Open **System Settings → System**. The build panel shows:

- the **version**, such as `0.2.0`;
- the **revision**: the source commit it was built from;
- the **image digest**: the exact image that is running;
- the **build date**.

What the version line says:

| It says | Meaning |
|---|---|
| `0.2.0` | A release build |
| `untagged build abc1234` | A build of commit `abc1234` that has no release version |
| `development build` | A local build, such as `make keel dev`. Revision, date and digest show as unknown |

If the digest shows as unknown, the deployment didn't tell Keel which image it runs. The
`keel_server` Ansible role sets it for you. Without the role, set `KEEL_IMAGE_DIGEST` yourself.

Who sees what:

| Role | Sees |
|---|---|
| Owner | The build panel and the update card |
| Admin | The build panel, and a line saying when a newer Keel is available. Nothing else on the tab |
| User | The version only, under **About**. No System tab |

## Turning update checks on

Checks are **off** by default. With checks off, Keel never contacts a registry.

Turn them on in the deployment settings. For a compose install, add these to `.env` next to
`compose.yaml`:

```env
KEEL_UPDATE_CHECK=true
# KEEL_UPDATE_INTERVAL=6h
# KEEL_UPDATE_IMAGE=docker.io/mbround18/keel:latest
```

With the `keel_server` Ansible role, set `keel_server_update_check: true` instead.

| Setting | Default | What it does |
|---|---|---|
| `KEEL_UPDATE_CHECK` | `false` | `true` turns checks on |
| `KEEL_UPDATE_INTERVAL` | `6h` | How often to check. At least `1h`. Accepts `90m`, `6h`, `1d` |
| `KEEL_UPDATE_IMAGE` | `docker.io/mbround18/keel:latest` | The image and tag to watch. Don't use a digest here |
| `KEEL_UPDATE_REGISTRY_USERNAME` | unset | A registry user, for a private image or a higher rate limit |
| `KEEL_UPDATE_REGISTRY_PASSWORD_FILE` | unset | A file holding that user's password or token |

Notes:

- Set the username and the password together, or neither.
- Keep the password in a file. In compose, put it in the secrets directory and name it by its
  path inside the container, such as `/run/keel/registry.password`.
- The password is never shown, logged or audited.
- A wrong setting stops Keel from starting. The error names the setting. The interval and image
  are checked even while checks are off.

Once on, Keel checks one or two minutes after it starts, then at the interval. Results live in
memory only. After a restart, the tab says "not checked yet" until the first check runs.

Checks are cheap. A check of an unchanged tag doesn't count against Docker Hub's pull limits. A
newly published image counts as one pull, once.

## What the update card says

Owners see the update card on the System tab. When an update is available, a dot also appears on
**System Settings** in the navigation.

| The card says | Meaning |
|---|---|
| Checks are off | `KEEL_UPDATE_CHECK` isn't `true` |
| Not checked yet | Checks are on. The first check hasn't run |
| Up to date | The watched tag points at the running image |
| Update available | The tag points at a different, newer image |
| Older build | The tag points at an image older than the running one. It isn't offered as an update |
| Rebuilt | The tag points at a new build of the same source revision |
| Can't tell | Keel can't compare. The card says why: for example, no running digest and no revision, or no image for this machine's architecture |

For an available image, the card shows its version, revision, build date and digest. A release
build links to its release notes. An untagged build says it has no release notes.

When a check fails, the card keeps the last good result. It shows the reason and when Keel will
try again. A failed check never shows an update as available. Common reasons:

| Reason | What to do |
|---|---|
| `rate_limited` | Wait. Or set registry credentials for a higher limit |
| `auth_failed` | Check the registry username and password file |
| `not_found` | Check `KEEL_UPDATE_IMAGE`: the image or tag doesn't exist |
| `network`, `timeout` | Check that Keel's host can reach the registry |

**Check now** runs a check at once. It works once a minute. Press it sooner and the card says when
you can try again.

**Dismiss** hides the dot for that image. It stays hidden after a reload. It comes back when a
different image is published. The card stays either way.

## Updating from the card

The **Update Keel** button needs all of these:

1. A System workspace. See [the System workspace](system-workspace.md).
2. Keel's own host enrolled in it, with the `keel-server` profile.
3. `keel-server` in the workspace's allowed profiles.
4. The `keel-server` job in the repository's `.keel/jobs.yaml` declaring the digest parameter.
   See [the rollout order](#rollout-order-for-the-digest-parameter).

To update:

1. Press **Update Keel**.
2. Read the preview. It names the host, the job, the running digest and the new one. It says
   that Keel will restart after the run reports.
3. Confirm with your passkey.
4. The run appears in **Runs**. It deploys exactly the digest you saw, never the tag. An image
   pushed after the preview isn't the one deployed.
5. Keel restarts on the new image. When it's back, the build panel shows the new digest.

Only one update runs at a time. Pressing Update again shows the run already in progress.

If the run fails before the restart, Keel keeps running the old image. The card links to the
failed run, and you can try again.

The audit log records each request, each refusal, the run's result, and the change of digest when
Keel starts on the new image.

## Updating by hand

When a prerequisite is missing, the card shows manual steps instead of the button. It names the
missing step, such as `no_system_workspace`, `no_keel_host`, `profile_not_allowed` or
`job_missing_param`. The steps include the exact image to deploy, like
`mbround18/keel@sha256:…`. Copy them from the card.

With the Ansible repository, deploy the digest from the card:

```sh
make keel-server KEEL_DIGEST=sha256:…
```

## Rolling back

The `keel_server` role saves the digest it replaced in `/opt/keel/digest.previous` on the Keel
host. To go back to it, run this in the Ansible repository:

```sh
make keel-server KEEL_DIGEST=$(cat /opt/keel/digest.previous)
```

Keel records the change of digest in the audit log when it starts.

`make keel-server` is also the repair path if Keel doesn't come back after an update.

## Rollout order for the digest parameter

The Update button passes the new digest to the `keel-server` job as a parameter. The job must
declare it:

```yaml
jobs:
  keel-server:
    params:
      keel_server_image_digest: { kind: oci_digest }
```

Add this line **last**. A Keel or runner without parameter support refuses a `jobs.yaml` that has
it. Do it in this order:

1. Land the `keel_server` role change in the Ansible repository. It pins the digest and writes
   `digest.previous`. It's safe at any time.
2. Update the runner on Keel's host: `make keel-node`.
3. Update Keel by hand once: `make keel-server`.
4. Now add `params:` to the `keel-server` job in `.keel/jobs.yaml`.
5. From then on, the Update button works.

Until step 4, the card shows the manual steps with `job_missing_param`. If you add `params:` too
early, the old Keel refuses the job. Nothing runs, but the `keel-server` job shows as broken until
you update Keel by hand.
