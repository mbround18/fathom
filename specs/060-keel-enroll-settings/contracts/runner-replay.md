# Contract: late run results after a Keel restart (spec 060, FR-037)

## Runner (`apps/keel/runner`)

1. **When sending fails**: if writing `Finished` to the channel fails, or the connection drops
   before the server's acknowledgement, the runner writes the frame to
   `<state_dir>/undelivered/<run_id>.json`. It writes a temporary file, fsyncs it and renames it,
   with mode 0600.
2. **After `Hello` is accepted**: the runner sends each file, oldest first, as a normal `Finished`
   frame.
   - It deletes the file on acknowledgement.
   - It keeps the file on error, and retries on the next connection.
3. **Expiry**: a file older than the maximum offline time (`KEEL_RUNNER_MAX_OFFLINE`, default the
   runner's existing limit) is deleted unsent, with one warning log line naming the run id.
4. **No secrets**: the frame holds only what `Finished` already holds.

## Server (`apps/keel/server`)

| `run_hosts` row | Action |
|---|---|
| running | Normal handling. |
| lost, `state_reason = runner_lost` | Record the outcome and the end time, recompute the run's state, audit `run_result_late`, emit the run event. |
| Already finished | Acknowledge and ignore. |
| Unknown run, or a host other than the sender's | Acknowledge, ignore, log a warning. Never trust it. |

## Tests

- **Runner**: spool, replay, expiry and file mode (unit tests with a temporary directory).
- **Server**: an integration test where a `runner_lost` row gets a late `Finished`, and a duplicate
  is ignored.
