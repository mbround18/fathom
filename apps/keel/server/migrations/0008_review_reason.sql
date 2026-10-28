-- Why a review group needs review (spec 055 US8, T100): what Keel's early check found, so a
-- reviewer sees it without re-running the check. Additive only.
alter table review_groups
    add column reason text not null default 'unsigned'
        check (reason in ('unsigned', 'downgrade', 'baseline', 'check_failed'));
