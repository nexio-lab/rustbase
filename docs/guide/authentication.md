# Authentication

RustBase supports four ways to authenticate, on top of a layered admin model. Pick whichever fits your client.

## Identities at a glance

| Principal | Lives in | Login endpoint |
|---|---|---|
| Master admin | `system.db` | `POST /_/auth/admin/login` (username + password) |
| Workspace admin | `<workspace>/workspace.db` | `POST /api/workspaces/:workspace/auth/admin/login` |
| App admin | `<workspace>/workspace.db`, scoped to apps | (same as workspace) |
| End-user | `<workspace>/workspace.db` | `POST /api/workspaces/:workspace/auth/users/login` |

End-users are **workspace-scoped**. A single `(email, workspace)` pair is one identity across every app in that workspace — sign in once with the workspace, hit any app inside it. Tokens carry the `(workspace, user_id)` tuple; the per-app target comes from the URL path (`/api/workspaces/:workspace/apps/:app/...`) rather than the token claim.

The master admin is created automatically on first boot with username `admin` and a NULL password. The setup wizard at `POST /_/setup` accepts a single `{ "password": "..." }` body to finish initialization.

Every login returns:

```json
{
  "access_token":  "ey...",       // JWT, 15-min TTL by default
  "refresh_token": "rfsh_...",    // opaque, rotates on every exchange
  "admin":         { ... }        // or "user": { ... }
}
```

Send `Authorization: Bearer <access_token>` on every authenticated call. Refresh tokens are exchanged at the matching `/auth/refresh` for the principal's scope.

::: tip JWT signing algorithm
Access tokens are signed with **RS256** (RSA-2048) by default. The
public verification key is published unauthenticated at:

```
GET /.well-known/jwks.json
GET /_/auth/jwks.json
```

Both routes return the same JSON Web Key Set. The `kid` field on the
JWT header matches a `kid` in the JWKS — standard JWT libraries
(jose, jsonwebtoken, oidc-client-ts, etc.) consume this format
without any custom configuration.

Servers upgraded from v0.1.x continue to accept HS256 tokens already
in flight; those naturally retire once the access-token TTL expires.
Newly-issued tokens are RS256.
:::

Refresh tokens are exchanged at the matching `/auth/refresh` for the principal's scope:

```http
POST /_/auth/refresh                               # master admin
POST /api/workspaces/:workspace/auth/refresh       # workspace / app admin
POST /api/workspaces/:workspace/auth/users/refresh # end-user
```

Each one accepts `{ "refresh_token": "rfsh_..." }` and returns a fresh access + refresh pair. The old refresh token is **invalidated immediately**.

## Email + password

The default end-user flow.

```http
POST /api/workspaces/:workspace/auth/users/register
{ "email": "u@acme.com", "password": "userpass1" }

POST /api/workspaces/:workspace/auth/users/login
{ "email": "u@acme.com", "password": "userpass1" }
```

Passwords are hashed with `argon2` by default. The minimum length and required character classes are configurable per scope through the `password.*` [policies](/concepts/hierarchical-policies).

### Email verification

```http
POST /api/workspaces/:workspace/auth/verify-email/request    [user token]
POST /api/workspaces/:workspace/auth/verify-email/confirm    { "token": "..." }
```

`request` mails the user a single-use token. `confirm` consumes it and sets `users.verified = true`. Tokens expire after one hour; calling `request` again invalidates earlier pending tokens.

### Password reset

```http
POST /api/workspaces/:workspace/auth/password-reset/request     { "email": "..." }
POST /api/workspaces/:workspace/auth/password-reset/confirm     { "token": "...", "new_password": "..." }
```

Same shape as verify-email: emailed one-shot token, single-use, hour TTL.

## Email OTP (passwordless)

For "magic link"-style flows.

```http
POST /api/workspaces/:workspace/auth/otp/request    { "email": "u@acme.com" }
POST /api/workspaces/:workspace/auth/otp/login      { "email": "u@acme.com", "code": "123456" }
```

`request` mails the user a 6-digit code valid for 10 minutes (configurable). `login` exchanges email + code for a fresh access/refresh pair. If the email doesn't exist yet, the user is auto-registered with no password — `users.has_password = false`. That's how you ship a 100% passwordless onboarding.

## TOTP (second factor)

Time-based one-time passwords via [`totp-rs`](https://docs.rs/totp-rs). Enrollment is a two-step dance so a misconfigured authenticator app can't accidentally lock the user out.

```http
POST /api/workspaces/:workspace/auth/totp/enroll       [user token]
  → returns { "secret": "...", "qr_url": "otpauth://..." }

POST /api/workspaces/:workspace/auth/totp/confirm      [user token]
  { "code": "123456" }
  → 200, TOTP is now active for this user

POST /api/workspaces/:workspace/auth/totp/disable      [user token]
  { "code": "123456" }
```

After enrollment, the regular login becomes a two-step flow:

```http
POST /api/workspaces/:workspace/auth/users/login
  { "email": "...", "password": "..." }
  → 202  { "mfa_required": true, "challenge_id": "..." }

POST /api/workspaces/:workspace/auth/users/login/totp
  { "challenge_id": "...", "code": "123456" }
  → 200  { access_token, refresh_token, user }
```

Workspace admins can force-unenroll a user with `DELETE /api/workspaces/:workspace/users/:id/totp` (recovery flow).

## OAuth2 / OIDC

Built on the [`oauth2`](https://docs.rs/oauth2) crate. Providers are configured **per workspace** by an admin — once enabled they apply to every app in the workspace:

```http
PUT /api/workspaces/:workspace/auth/oauth/providers/google
{
  "client_id":     "...",
  "client_secret": "...",       // optional on update; preserved if omitted
  "config": {
    "auth_url":     "https://accounts.google.com/o/oauth2/v2/auth",
    "token_url":    "https://oauth2.googleapis.com/token",
    "userinfo_url": "https://openidconnect.googleapis.com/v1/userinfo",
    "scopes":       ["openid", "email", "profile"]
  }
}
```

The dashboard ships with **Google**, **GitHub**, and **Microsoft** presets — pick one and only the client id/secret need filling in.

`client_secret` is encrypted at rest under the server's KEK and **never** echoed back on read. To rotate it, PUT with a new secret; to keep the existing one, PUT without `client_secret`.

### End-user flow

```http
GET /api/workspaces/:workspace/auth/oauth/google/authorize?redirect_uri=https://app/cb
  → 200 { authorize_url, state }
  The authorize URL carries client_id, redirect_uri, scope,
  response_type=code, state, code_challenge, code_challenge_method=S256.

POST /api/workspaces/:workspace/auth/oauth/google/callback
  { "code": "...", "state": "..." }
  → 200  { access_token, refresh_token, user }
```

State is single-use and bound to the workspace. On first sign-in via
OAuth, a user row is created in `workspace.db` with
`has_password = false`. Subsequent logins match by `email` within the
same workspace, so the user can hit any app in it with one identity.

::: tip PKCE everywhere
Every flow is **PKCE-protected (RFC 7636, S256)** — the server mints
a 32-byte `code_verifier` at `/authorize`, stores it alongside the
state nonce, sends `code_challenge` to the provider, and replays the
verifier on the token exchange. Providers that don't honour PKCE
ignore the parameters; providers that do (Google, GitHub, Microsoft,
every modern OIDC IdP) refuse the token exchange when the verifier
doesn't match the original challenge. Nothing to configure.
:::

## Admin tokens

Master admins call `/_/auth/admin/login` with `{ username, password }`; workspace admins call `/api/workspaces/:workspace/auth/admin/login` with `{ email, password }`. Tokens carry a `role` claim (`master_admin`, `workspace_admin`, `app_admin`, `user`) plus the scope claims that match it: master admin tokens have neither `workspace` nor `app`; workspace-admin tokens carry `workspace`; app-admin tokens carry both `workspace` and `app`; end-user tokens always carry both `workspace` and `app` (users are per-app since the users-per-app refactor). Every protected handler enforces:

```rust
auth.require_master()?;             // master only
auth.require_realm_access(workspace)?;  // master OR workspace admin for this workspace
auth.require_app_access(workspace, app)?; // master OR workspace admin OR app admin scoped here
```

Tokens are stateless. Revocation lives in an in-memory `DashSet` checked by middleware and auto-expires after the access-token TTL.

## Dashboard session cookies

The embedded dashboard authenticates with `HttpOnly` session cookies
rather than reading the JWT directly. Two cookies are set on every
successful master / workspace / user login response (and on every
`/auth/refresh`):

| Cookie  | Path       | Max-Age          | Notes |
|---------|------------|------------------|-------|
| `rb_at` | `/`        | access TTL (15 min) | Sent on every same-origin call so the REST surface authenticates implicitly. |
| `rb_rt` | `/_/auth`  | refresh TTL (30 days) | Scoped so the refresh token only travels to dashboard auth endpoints. |

Both cookies are `HttpOnly` + `SameSite=Strict`. JS in the dashboard
cannot read them — an XSS bug in a rendered cell can no longer
exfiltrate the user's tokens, and `SameSite=Strict` blocks the
cross-site CSRF vector without a separate token.

A flag `[http].cookie_secure` controls whether the `Secure`
attribute is emitted. Defaults to `true` (production behind TLS); set
to `false` for local-dev HTTP — browsers reject `Secure` cookies on
plain origins.

`POST /_/auth/logout` clears both cookies (`Max-Age=0`) and revokes
the refresh token server-side. The endpoint is anonymous: a session
that's lost its access token can still log out cleanly.

The Bearer header path stays fully supported — SDK clients,
server-to-server callers, and CLI tools keep using `Authorization:
Bearer <jwt>` unchanged.

## TTL policies

`tokens.access_ttl_sec` and `tokens.refresh_ttl_sec` are [hierarchical policies](/concepts/hierarchical-policies). Default access TTL is 15 minutes, default refresh TTL is 30 days. Master sets the bounds; workspaces (and apps) tighten.
