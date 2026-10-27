# Research: Uwusquad Suite Integration Strategy

## Goal

Unify Fathom, Depths, Keel, and Ballast into one secure product suite at `uwusquad.dev`, with cross-app workflows, shared RBAC, and consistent branding.

## External Findings (authoritative references)

1. **NATS JetStream is built for durable, replayable event workflows**
   - JetStream provides streams, stateful consumers, explicit acknowledgements, replay, and at-least-once delivery, which maps well to cross-app orchestration and recovery.
   - Source: NATS docs (`docs.nats.io/concepts/jetstream`).

2. **Modern OAuth/OIDC deployments need current security controls**
   - OAuth security best current practice updates older assumptions and addresses multi-tenant/dynamic client realities.
   - Source: IETF RFC 9700 (OAuth 2.0 Security BCP).

3. **Centralized policy decision + distributed enforcement is a strong suite pattern**
   - OPA’s decision/enforcement split is a practical model for consistent authorization across multiple products.
   - Source: OPA docs (`openpolicyagent.org/docs`).

## Recommended Suite Architecture

## 1) Identity and Access Plane

- Every product runs standalone by default; suite membership is a per-product opt-in (see spec "Standalone and Suite Mode").
- Shared organization/team model consumed by enrolled products.
- Central policy authoring in the portal, distributed as signed bundles (OPA-bundle style); each product makes the decision and enforces it locally, never calling the portal per request.
- Immediate revocation propagation (push + bundle max age as the upper bound).

## 2) Workflow and Event Plane

- A shared suite event contract for:
  - workflow requested
  - step started/completed/failed
  - approval requested/granted/denied
  - rollback started/completed/failed
- Product adapters translate suite events into each app’s native command/event protocol.
- Idempotency keys and deduplication for retry-safe execution.

## 3) Trust and Audit Plane

- Immutable audit chain across all products.
- Every cross-app step stores actor, team scope, policy decision, and result.
- Unified search by actor/team/product/workflow/time.

## 4) Product and UX Plane

- Portal as the top-level entry point:
  - shared nav, shared session, shared policy context
  - product cards with purpose and allowed actions
  - cross-app workflow catalog
- Product boundaries remain explicit to avoid operator confusion.

## Branding Recommendations (you asked for help here)

- Brand hierarchy:
  - **Uwusquad** = suite brand
  - **Fathom / Depths / Keel / Ballast** = product brands
- Keep existing nautical motif and marks as product-level identifiers.
- Add a suite style guide that defines:
  - tone and voice (operator-trust, not playful in critical UX)
  - icon grammar
  - color and state semantics
  - copy rules for sensitive actions and approvals
- Keep “cute/anime” identity to marketing surfaces, but keep operational UI language sober for trust and compliance.

## Cross-App Workflow Examples to Prioritize

1. **Keel → Ballast → Depths → Fathom**
   - Provision host/inventory, apply infrastructure config, deploy runtime stack, verify workloads.

2. **Fathom → Ballast**
   - Detect cluster state signal (e.g., Flux condition), trigger approved remote configuration workflow.

3. **Keel → Depths**
   - Provision game server host path, then deploy game stack through managed templates.

## Risks and Mitigations

- **Policy drift between products**
  - Mitigation: one policy source, automated policy conformance checks in each product.
- **Workflow partial failures**
  - Mitigation: mandatory rollback metadata per step, compensating actions, and resumable runs.
- **Privilege escalation across products**
  - Mitigation: scoped delegation grants + explicit approval gates for sensitive cross-product actions.
- **Brand inconsistency**
  - Mitigation: shared branding package and suite-wide release checks for product identity surfaces.
