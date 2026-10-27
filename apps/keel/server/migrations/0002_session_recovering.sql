-- Spec 055 T028: a session that redeemed a recovery code may enrol a passkey without a prior
-- check, until it does (054 R12).
alter table sessions add column recovering boolean not null default false;
