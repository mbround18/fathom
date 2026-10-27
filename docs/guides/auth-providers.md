# Suite Auth Providers (Docs-First Matrix)

This suite uses a shared provider model for sign-in and group-based authorization mapping. Start
with standards-based OIDC setup and claim mapping. Add provider-specific work only when operators
report a concrete issue.

## Support intent

- **Primary baseline**: OIDC discovery + authorization code + PKCE + group claims mapping.
- **First-class starting point**: Microsoft Entra ID.
- **Also targeted**: other major providers where OIDC/group claims are available.
- **Fail-closed rule**: if required claims are missing or malformed, access is denied with
  actionable setup guidance.

## Provider reading list

| Provider | Primary docs |
|---|---|
| Microsoft Entra ID | https://learn.microsoft.com/entra/identity-platform/v2-protocols-oidc |
| Google Identity Platform | https://developers.google.com/identity/openid-connect/openid-connect |
| Okta | https://developer.okta.com/docs/guides/implement-grant-type/authcodepkce/main/ |
| Auth0 | https://auth0.com/docs/authenticate/protocols/openid-connect-protocol |
| AWS Cognito | https://docs.aws.amazon.com/cognito/latest/developerguide/cognito-userpools-oidc-flow.html |
| Keycloak | https://www.keycloak.org/documentation |
| GitHub OIDC/OAuth | https://docs.github.com/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps |
| GitLab OIDC | https://docs.gitlab.com/integration/openid_connect_provider/ |

## Group/role mapping notes

- Prefer stable group identifiers where providers emit IDs instead of names.
- Document provider-specific overage behavior (for example, large group membership responses).
- Keep mappings explicit and auditable: provider claim -> suite team/role binding.
- Do not grant fallback implicit access when group claims are absent.

## Existing related docs in this repository

- Fathom hosted OIDC and group claim guidance: `docs/HOSTED.md`
- Depths provider setup: `docs/guides/depths/sign-in.md`
- Suite architecture and positioning: `docs/UWUSQUAD.md`
