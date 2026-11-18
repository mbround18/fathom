-- Spec 067 US3 (T030): an owner may remove a zone. 0017 revoked `delete` on zones; the
-- foreign keys from hosts and relays (no cascade) still refuse removing a zone that any host
-- or relay, revoked ones included, points at, and the handler checks first and lists them.
--
-- 0019 is reserved for another branch; sqlx applies unapplied versions in order.

grant delete on zones to keel_app;
