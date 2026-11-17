-- Spec 067 D2: one run per host. `queue::dispatch_ready` never claims a queued row for a host
-- that has one in `sent` or `running`; this index is the backstop.

-- Rows from before the lock: a host with more than one in flight keeps its oldest, and the rest
-- become `lost` (no session survives a restart). A runner that replays its `Finished` still
-- records the real outcome on a `lost` row (spec 060 FR-037).
update run_hosts rh
   set state = 'lost', ended_at = now()
  from (select q.run_id, q.host_id,
               row_number() over (partition by q.host_id
                                  order by r.created_at, r.id) as n
          from run_hosts q join runs r on r.id = q.run_id
         where q.state in ('sent', 'running')) d
 where rh.run_id = d.run_id and rh.host_id = d.host_id and d.n > 1;

create unique index run_hosts_one_in_flight on run_hosts (host_id)
    where state in ('sent', 'running');
