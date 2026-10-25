# Protocol additions (052)

- `GET /api/me` adds `network: TrustDecision`, `needs_answer: bool`, admins: `proxy_warning: bool`.
- Refusal kind `untrusted_network { network, reason, matures_at? }` on WS and HTTP (HTTP 403 JSON).
- `GET /api/sessions` → `SessionView[] { id_hint, created, last_used, address, network, trusted, location, browser, current }`.
- `DELETE /api/sessions/{id_hint}`, `POST /api/sessions/end-others` — allowed from any network; CSRF required.
- `GET /api/networks` → `NetworkView[]` (KnownNetwork + `trusted`, `location`); `POST /api/networks/answer { network, answer }`.
- Admin: `GET /api/users/{id}/sessions|networks`, `DELETE /api/users/{id}/sessions` (any network); `POST /api/users/{id}/networks/whitelist { cidr, label? }`, `DELETE /api/users/{id}/networks/{key}` (trusted network only; refused if the CIDR contains the requester's current untrusted network).
- `id_hint` is a non-reversible short hash of the session handle; never the cookie.
- ts-rs bindings regenerated.
