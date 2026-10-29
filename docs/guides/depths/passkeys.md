# Passkeys

Signing in through your provider proves who you are. For the actions that could hurt the host,
Depths also asks for a passkey: a security key, or the fingerprint, face or PIN unlock built into
your phone or computer. Someone who steals your provider session still can't open a shell or
change files without your device.

## Registering a passkey

Open **Your sessions** from the account menu. The **Your passkeys** panel lists your passkeys with
the date each was added and last used. Choose **Add passkey** and follow your browser's prompt, then give
the passkey a name you'll recognise later, like "work laptop" or "yubikey".

You don't have to go looking for it the first time: when an action needs a passkey and you have
none, Depths opens the same dialog.

Register at least two, on different devices. The last passkey can't be removed, and losing every
one means asking the operator for a recovery code (below).

## Confirming it's you

**Admins** confirm once per session. Until then, a bar at the top reads **Admin actions locked**,
and admin actions are refused. Choose **Confirm it's you** and use your passkey. An admin with no
passkey sees **Register a passkey** there instead, and registers one before any admin action.

## Risky actions ask again

These actions ask for a passkey, whatever your role, unless you already used it within the last
few minutes:

- opening a shell in a container;
- writing, uploading, renaming or deleting files in a volume, and restoring a file's earlier
  version;
- changing people, roles or network whitelists;
- changing update settings: updating several stacks at once, Auto policies, the Watchtower
  acknowledgement and image cleanup (see [Image updates](updates.md));
- removing a passkey;
- starting, restarting, updating or saving a stack whose compose file has escape-level findings
  (privileged mode, the Docker socket, host namespaces and the like).

The passkey prompt is part of the confirm step you already see. Cancelling it, or a failed check,
changes nothing.

A passkey doesn't replace network trust: on an untrusted network these actions stay refused even
with a passkey (see [Networks and the restricted view](security.md)).

### The window

After a passkey check, risky actions go through without asking again for 5 minutes. The operator
sets this with `DEPTHS_STEP_UP_WINDOW`, as seconds (`120`) or a duration (`2m`). It's kept between
30 seconds and 1 hour: a value outside that is clamped, and Depths logs a warning at start-up
naming the value it used.

## Removing a passkey

Choose **Remove** next to a passkey in **Your passkeys** (the bin icon on a phone), then **Remove** again in the dialog. Depths asks for one of
your passkeys to confirm. You can't remove your only passkey; add another first.

## Lost every passkey: the recovery code

Recovery needs someone with access to the host, so a stolen provider account alone can't use it.
The operator runs this next to the compose file, with the person's id (`provider:subject`, the
same id shown in [People and access](users.md)):

```sh
docker compose exec depths depths passkey-recovery --user github:12345678
```

It prints a 10-character code. The code:

- works once, for that person only;
- expires after 15 minutes;
- replaces any earlier code that hasn't been used.

The person enters it under **Lost every passkey?** in **Your passkeys** and chooses **Use code**,
then registers a new passkey straight away. Depths stores only a hash of the code and never writes
the code itself to the audit log or the logs.

## After upgrading to a Depths with passkeys

Nobody has a passkey after the upgrade. Each admin's first visit shows **Admin actions locked**
with **Register a passkey**, and admin actions stay refused until they register. Everyone else
registers the first time they try a risky action. Have each admin register right after the
upgrade, before they need it in a hurry.

## Changing Depths' address

Passkeys are tied to the host name in `DEPTHS_PUBLIC_URL`. If that host name changes, for example
from `depths.example.com` to `stacks.example.com`, every existing passkey stops working, and
everyone registers again. Admins without a working passkey need a recovery code, so plan the
change with host access to hand. Changing only the path or port of the URL doesn't affect
passkeys if the host name stays the same.

## What's recorded

Every registration, removal, confirmation, failed confirmation and recovery is in the audit log,
with who did it and when. Passkey credentials, challenges and recovery codes never are.
