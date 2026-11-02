# Roadmap: private PKI for the secret manager

**Recorded**: 2026-11-02, from the owner's direction. **Status**: roadmap, not a spec. It extends
spec 059, whose first version is ACME-only (see its Assumptions). Each phase below becomes its own
spec when 059 is planned.

## Why

The suite apps (Keel, Depths, Ballast, Fathom's server and daemon) need TLS they can trust without a
public name, mutual TLS between each other, and a safe way to move credentials. Let's Encrypt covers
public and split-horizon names. It does not cover service identity, mTLS, names nobody may publish,
or an enterprise that already runs its own certificate authority.

## What the owner wants

1. **Build a root CA.** The secret manager creates a root and issues certificates for the suite
   apps and anything else that needs tight security.
2. **Strong cryptography.** Modern algorithms by default, with room for what comes next.
3. **Kubernetes issuance through Fathom.** The in-cluster daemon issues certificates for workloads,
   and Fathom shows and manages them.
4. **Safe transport of secrets.** Credentials move encrypted end to end, not just over TLS: SOPS
   for anything at rest in git, the suite's own TLS for the wire.
5. **Import an enterprise root.** An organisation that already runs a CA can bring it, and the
   secret manager works under it.
6. **Many roots, many tenants.** Multiple CAs, owned per workspace, with the same governance as
   Depths and Keel.

## Shape

### Hierarchy and keys

- A **root** per tenant, used only to sign intermediates. Its key is sealed and never used online
  except under a two-person, step-up ceremony. It can also be offline: export the root key sealed,
  and keep only the intermediates in the service.
- **Intermediates** do the issuing, each with name constraints (the domains, IPs and SPIFFE-style
  URIs it may sign) and an issuance policy: allowed key types, maximum lifetime, who may request.
- **Algorithms**: ECDSA P-384 for roots and intermediates, ECDSA P-256 or Ed25519 for leaves where
  clients accept it, RSA-3072/4096 only for clients that need it. Track hybrid post-quantum
  signatures (ML-DSA) and add them as an option when rustls and the browsers support them. Key
  generation and signing behind a port, so a PKCS#11 HSM, a TPM or a cloud KMS is an adapter.
- **Short-lived leaves** by default (days, not years), renewed automatically, so revocation is the
  exception. CRL and OCSP for the cases where it is needed.

### Issuance paths

- **CSR first**: the consumer makes its own key and sends a CSR. The private key never leaves the
  machine or pod that uses it. Issuing a key for someone else is allowed only for consumers that
  cannot make one, and is audited as such.
- **An internal ACME endpoint**, so anything that speaks ACME (Caddy, Traefik, cert-manager,
  certbot) gets suite certificates the same way it gets public ones.
- **App enrollment**: a suite app joins with a one-time enrollment token and a fingerprint the
  operator confirms, then holds an mTLS identity it renews on its own.

### Kubernetes, through Fathom and the daemon

- The daemon acts as an issuer inside the cluster: a cert-manager external issuer, and a signer for
  the Kubernetes CSR API (`certificates.k8s.io`). Workloads keep their keys; the daemon forwards
  CSRs to the secret manager under that cluster's grant.
- Trust bundles (the tenant's roots) are distributed to namespaces as ConfigMaps, the way
  trust-manager does, so workloads trust suite certificates without baking them into images.
- Fathom shows every certificate in the cluster: chain, issuer, names, expiry, renewal state, which
  workload mounts it. Manifest export as everywhere else; nothing is a black box.
- Delivery of plain secrets into namespaces stays as spec 059 US3 describes.

### Moving secrets safely

- **On the wire**: mTLS between suite apps, pinned to the tenant's root, on top of the normal TLS.
- **End to end**: a secret value is sealed to the receiving consumer's key (HPKE or age) before it
  leaves the secret manager, so a proxy or a log in between sees only ciphertext.
- **At rest in git**: SOPS with age recipients (Ballast already tracks SOPS lineage). Export and
  import between instances seal to the destination's key, never in clear (059 FR-015).

### Enterprise roots

Three ways to bring an existing CA, in order of how much the enterprise trusts us:

1. **Trust only**: import the root certificate (no key) to verify and to add to trust bundles.
2. **Subordinate**: the enterprise signs an intermediate for us, with name constraints; we issue
   under it and its key lives here.
3. **External signer**: the enterprise keeps every key; an adapter forwards our CSRs to their CA
   (AD CS, HashiCorp Vault PKI, EJBCA, step-ca) and records what came back.

### Tenancy and governance

- CAs belong to a workspace, like secrets. A tenant sees and issues only from its own CAs.
- Several roots per tenant are allowed (one per environment, or during a root rotation).
- Root rotation with overlap: a new root cross-signed by the old, both in trust bundles until every
  leaf has renewed.
- Creating, importing or rotating a CA needs step-up and a second approver. Every issuance,
  renewal and revocation goes into the hash-chained audit, with an issuance log a tenant can
  export.

## Phases

1. Private root and intermediate per tenant, CSR issuance, short-lived leaves, audit. Keel and
   Depths get mTLS identities through enrollment.
2. Internal ACME endpoint, trust bundles, CRL and OCSP.
3. Daemon as cert-manager issuer and CSR signer; Fathom's certificate inventory.
4. Enterprise import (trust only, subordinate, external signer).
5. HSM, TPM and KMS adapters; post-quantum hybrid option.

## UI the kit should have first

So that building this is mostly logic: a masked secret field (reveal on step-up, copy), an expiry
badge (valid, renewing, expiring, expired), a certificate chain view, a description list for detail
panes, a PEM/file drop field with validation, a key-value editor, a filter bar, an audit timeline,
and a read-only PEM/YAML viewer.

## Open questions

- Is the root kept online (sealed, ceremony-gated) by default, or offline by default?
- SPIFFE IDs for workload identity, or names only?
- Which enterprise CA to support first for the external-signer adapter?
- Does the secret manager itself run in Kubernetes, beside Keel, or both?
