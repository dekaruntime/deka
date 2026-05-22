# deka.gg SSO integration

deka.gg uses NextAuth with the Tana SSO provider defined in `lib/sso.ts`.
Unauthenticated requests are redirected by `middleware.ts` to `/signin`.
That page immediately calls `signIn('tana')`, which starts the OAuth flow
against id.tana.gg.

## Runtime configuration

Production deploys must set:

```bash
NEXTAUTH_URL=https://deka.gg
NEXTAUTH_SECRET=<existing NextAuth secret>
SSO_IDP_URL=https://id.tana.gg
SSO_SECRET_DEKA=<operator-provisioned shared secret>
```

Do not generate `SSO_SECRET_DEKA` in application code or in this repo. The
secret is an operator-managed deploy value and must match the `deka.gg`
`:Service` record on id.tana.gg, where `hmac_secret_ref` is `SSO_SECRET_DEKA`.

`SSO_CLIENT_SECRET` remains supported as a local-development fallback for older
env files, but production should use `SSO_SECRET_DEKA`.

## OAuth flow

1. A browser requests a protected deka.gg page.
2. Middleware checks the NextAuth JWT session cookie.
3. Without a valid session, middleware redirects to `/signin`.
4. The sign-in page calls NextAuth's Tana provider.
5. NextAuth redirects to `GET https://id.tana.gg/auth/sso/authorize` with
   `client_id=deka.gg`, `scope=openid email profile`, PKCE, state, and the
   NextAuth callback URL.
6. id.tana.gg authenticates the user and redirects back to deka.gg.
7. NextAuth exchanges the authorization code at `/auth/sso/token`, loads
   `/auth/userinfo`, and writes the session cookie.

## Callback URL note

NextAuth v4's standard callback for this provider is:

```text
https://deka.gg/api/auth/callback/tana
```

The current id.tana.gg `:Service` record was populated with:

```text
https://deka.gg/auth/sso/callback
```

For the first canary, deka.gg keeps the native NextAuth callback path instead
of adding a custom callback shim. Operators should add
`https://deka.gg/api/auth/callback/tana` to the `deka.gg` service
`allowed_return_uris` before enabling the production canary. A later shared
NextAuth provider package can revisit a custom callback path if all consumers
need vanity callback URLs.

## Local canary test

Run:

```bash
bun run test:e2e
```

The Playwright test starts deka.gg locally with `SSO_IDP_URL` pointed at a mock
id.tana.gg server. It verifies that a protected page redirects through the SSO
authorize endpoint, completes a mocked OAuth callback, and receives a NextAuth
session cookie.
