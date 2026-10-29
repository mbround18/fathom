-- Spec 061: a host refuses a run whose job params don't match what the job declares at the target
-- commit (`keel_exec::frames::Refusal::ParamsMismatch`). Additive: every older value stays valid.
alter table run_hosts drop constraint run_hosts_refusal_check;
alter table run_hosts add constraint run_hosts_refusal_check check (refusal in (
    'unsigned', 'bad_override', 'base_mismatch', 'profile_not_allowed', 'downgrade',
    'replay', 'ledger_corrupt', 'version_too_old', 'params_mismatch'
));
