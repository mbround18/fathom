-- Spec 067 T066 F7 (owner decision 2026-11-17, "quarantine after auth only"): when the relay's
-- pinned host key first completed an authenticated link (SSH with the pinned key, Keel's
-- relay-link key accepted, and a `Hello` naming that key). Until then a host-key mismatch is a
-- lost link (backoff, coalesced `relay.refused`), not a quarantine, so an attacker on the network
-- path cannot take a never-proven relay out of service. Cleared when an owner changes the host
-- key. Persisted so a restart of Keel does not reopen the hole.

alter table relays add column host_key_verified_at timestamptz;
