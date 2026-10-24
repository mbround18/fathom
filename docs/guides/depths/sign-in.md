# Sign-in providers

Everyone signs in to Depths through one of three providers: an OIDC identity provider, GitHub, or
Discord. There is no anonymous mode and no local password: with no provider configured, Depths
refuses to start.

You can configure more than one. The sign-in page shows one button per configured provider.

Signing in only proves who someone is. It does not give them access: a new person is recorded as
pending until an admin lets them in (see [People and access](users.md)).

## The redirect URI

Every provider asks for a redirect (callback) URL. For Depths it is always:

```
<DEPTHS_PUBLIC_URL>/auth/callback/<provider>
```

so with `DEPTHS_PUBLIC_URL=https://depths.example.com`:

| Provider | Redirect URI |
|---|---|
| OIDC | `https://depths.example.com/auth/callback/oidc` |
| GitHub | `https://depths.example.com/auth/callback/github` |
| Discord | `https://depths.example.com/auth/callback/discord` |

## Two ways to name the credentials

**One provider, short names.** Set `OAUTH_PROVIDER` to `oidc`, `github` or `discord`, and put its
credentials in `OAUTH_CLIENT_ID` and `OAUTH_CLIENT_SECRET`. These are the names the example `.env`
uses, and the names a Dockge-era `.env` may already have. Setting `OAUTH_CLIENT_ID` or
`OAUTH_CLIENT_SECRET` without `OAUTH_PROVIDER` stops start-up.

**Any number of providers, full names.** Each provider has its own variables, and they win over
the short names:

| Provider | Variables |
|---|---|
| OIDC | `DEPTHS_OIDC_ISSUER`, `DEPTHS_OIDC_CLIENT_ID`, `DEPTHS_OIDC_CLIENT_SECRET`, `DEPTHS_OIDC_SCOPES` |
| GitHub | `DEPTHS_GITHUB_CLIENT_ID`, `DEPTHS_GITHUB_CLIENT_SECRET` |
| Discord | `DEPTHS_DISCORD_CLIENT_ID`, `DEPTHS_DISCORD_CLIENT_SECRET` |

Any secret can be read from a file instead: set `DEPTHS_GITHUB_CLIENT_SECRET_FILE=/run/secrets/gh`
(or `OAUTH_CLIENT_SECRET_FILE`) rather than the secret itself. Setting both a variable and its
`_FILE` stops start-up.

## OIDC

Use this with Keycloak, Authentik, Rauthy, Dex, Zitadel, Entra ID, Okta or any other provider that
supports OpenID Connect discovery.

1. In your identity provider, create a confidential client (or a public one, see below) with the
   redirect URI `https://depths.example.com/auth/callback/oidc`.
2. Set:

   ```sh
   DEPTHS_OIDC_ISSUER=https://idp.example.com/realms/ops
   DEPTHS_OIDC_CLIENT_ID=depths
   DEPTHS_OIDC_CLIENT_SECRET=<client secret>
   ```

   or, with the short names, `OAUTH_PROVIDER=oidc`, `OAUTH_CLIENT_ID`, `OAUTH_CLIENT_SECRET` and
   `DEPTHS_OIDC_ISSUER` (the issuer has no short name).

Details:

- Depths reads the issuer's discovery document at start-up. If the issuer cannot be reached, Depths
  does not start, and the message names the issuer.
- Scopes default to `openid profile email`. Set `DEPTHS_OIDC_SCOPES` (space- or comma-separated) to
  change them; `openid` is always sent.
- The client secret is optional. Sign-in always uses PKCE, so a public client works without one.
- A person is identified by the token's `sub` claim. Their display name comes from
  `preferred_username`. Groups are not used: access is given in Depths, not by the provider.
- To make someone an owner, use `oidc:<sub>` (see [People and access](users.md)).

## GitHub

Depths uses a GitHub **OAuth App** (not a GitHub App).

1. Go to GitHub, **Settings > Developer settings > OAuth Apps > New OAuth App**. For an app owned by
   an organization, use the organization's settings instead.
2. Set **Homepage URL** to `https://depths.example.com` and **Authorization callback URL** to
   `https://depths.example.com/auth/callback/github`.
3. Register it, then **Generate a new client secret**.
4. Set:

   ```sh
   OAUTH_PROVIDER=github
   OAUTH_CLIENT_ID=<client id>
   OAUTH_CLIENT_SECRET=<client secret>
   ```

Details:

- Depths asks for the scopes `read:user` and `user:email`: your profile and your primary verified
  email. Nothing else.
- A person is identified by their numeric GitHub user id, not their login, because logins can be
  renamed and then taken by someone else.
- Depths makes no organization or team calls. It does not read your organizations, so an
  organization's OAuth App access policy (the setting that makes members request approval for
  third-party apps) has nothing to block. You do not need to request or grant organization
  approval for Depths, and approving it would give it nothing extra.
- Every 15 minutes Depths checks the session with GitHub. If you revoke the app in your GitHub
  settings, your Depths session ends at the next check.

## Discord

1. Go to the Discord Developer Portal and create an application.
2. Under **OAuth2**, add the redirect `https://depths.example.com/auth/callback/discord`, and copy
   the client id and client secret.
3. Set:

   ```sh
   OAUTH_PROVIDER=discord
   OAUTH_CLIENT_ID=<client id>
   OAUTH_CLIENT_SECRET=<client secret>
   ```

Details:

- Depths asks for the `identify` scope only. It does not ask for your email, your servers or your
  roles.
- A person is identified by their Discord user id (a long number).
- The application does not need a bot, and does not need to be in any server.

## When sign-in fails

- **"redirect_uri mismatch" or similar at the provider.** The redirect URI registered there must
  match `DEPTHS_PUBLIC_URL` exactly, including `https://` and with no trailing path.
- **You sign in and see "You don't currently have access".** Sign-in worked. You are pending:
  an admin has to let you in, or your id is not the one in `OWNER_USER_ID`. See
  [People and access](users.md).
- **Depths will not start.** The log names the setting. Common ones: no provider configured, a
  client id without its secret, an owner whose provider is not configured.
