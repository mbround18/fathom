# Uwusquad Suite Positioning

Uwusquad is the product-suite umbrella for:

- **Fathom**: Kubernetes operations
- **Depths**: Docker/Compose host operations
- **Keel**: Git-bound Ansible and inventory control plane
- **Ballast**: IaC state + secrets control plane
- **Uwusquad Portal** (`澪`): shared identity, RBAC, and cross-app workflow orchestration

## Product fit

Use `uwusquad.dev` as the single entry point:

- `uwusquad.dev/` — suite portal, onboarding, and workflow catalog
- `uwusquad.dev/fathom` — Kubernetes operations
- `uwusquad.dev/depths` — Compose host operations
- `uwusquad.dev/keel` — Ansible control plane
- `uwusquad.dev/ballast` — IaC state/secrets control plane

Every product runs standalone by default, with its own sign-in, sessions, authorization and audit
log. Joining the suite is an opt-in per product. Enrolled products keep their domain-specific UX and
their own sessions on their own origin, and additionally share:

- single sign-on through a shared identity provider (the portal is not a required broker)
- one team/role/scope policy model, written in the portal and sent to products as signed bundles;
  each product still decides every request itself
- one aggregated audit surface, fed from each product's own immutable log
- one cross-app workflow model that calls each product's public API
- one provider model for OIDC sign-in and group-based authorization mapping

`uwusquad.dev/<product>` routes are product pages, not where the products run. See spec 057,
"Standalone and Suite Mode", for the rules.

## Commercial posture

- **Free and open source core** across all products.
- **No paywall for core orchestration workflows.**
- Donation-first sustainability through approved sponsorship channels in `FUNDING.md`.

## Security posture

The suite uses a shared policy plane and enforces least privilege, deny-by-default authorization,
and immutable auditing for all cross-app workflows. Sensitive actions use explicit approval gates
and rollback-aware workflow execution.

Provider guidance lives in `docs/guides/auth-providers.md` and product-specific setup docs.
