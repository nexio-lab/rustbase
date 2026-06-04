# Authentication

RustBase supports four ways to authenticate, on top of a layered admin model. Pick whichever fits your client.

## Identities at a glance

| Principal | Lives in | Login endpoint |
|---|---|---|
| Master admin | `system.db` | `POST /_/auth/admin/login` (username + password) |
| Realm admin | `<realm>/realm.db` | `POST /api/realms/:realm/auth/admin/login` |
| App admin | `<realm>/realm.db`, scoped to apps | (same as realm) |
| End-user | `<realm>/apps/<app>/data.db` | `POST /api/realms/:realm/apps/:app/auth/users/login` |

End-users are scoped to one app. The same email address can register independently in two apps under the same realm — they're separate identities. Tokens carry the `(realm, app, user_id)` tuple.

The master admin is created automatically on first boot with username `admin` and a NULL password. The setup wizard at `POST /_/setup` accepts a single `{ "password": "..." }` body to finish initialization.

Every login returns:

```json
{
  "access_token":  "ey...",       // JWT, 15-min TTL by default
  "refresh_token": "rfsh_...",    // opaque, rotates on every exchange
  "admin":         { ... }        // or "user": { ... }
}
```

Send `Authorization: Bearer <access_token>` on every authenticated call. Refresh tokens are exchanged at the matching `/auth/refresh` for the principal's scope:

```http
POST /_/auth/refresh                              # master admin
POST /api/realms/:realm/auth/refresh              # realm / app admin
POST /api/realms/:realm/apps/:app/auth/users/refresh  # end-user
```

Each one accepts `{ "refresh_token": "rfsh_..." }` and returns a fresh access + refresh pair. The old refresh token is **invalidated immediately**.

## Email + password

The default end-user flow.

```http
POST /api/realms/:realm/apps/:app/auth/users/register
{ "email": "u@acme.com", "password": "userpass1" }

POST /api/realms/:realm/apps/:app/auth/users/login
{ "email": "u@acme.com", "password": "userpass1" }
```

Passwords are hashed with `argon2` by default. The minimum length and required character classes are configurable per scope through the `password.*` [policies](/concepts/hierarchical-policies).

### Email verification

```http
POST /api/realms/:realm/apps/:app/auth/verify-email/request    [user token]
POST /api/realms/:realm/apps/:app/auth/verify-email/confirm    { "token": "..." }
```

`request` mails the user a single-use token. `confirm` consumes it and sets `users.verified = true`. Tokens expire after one hour; calling `request` again invalidates earlier pending tokens.

### Password reset

```http
POST /api/realms/:realm/apps/:app/auth/password-reset/request     { "email": "..." }
POST /api/realms/:realm/apps/:app/auth/password-reset/confirm     { "token": "...", "new_password": "..." }
```

Same shape as verify-email: emailed one-shot token, single-use, hour TTL.

## Email OTP (passwordless)

For "magic link"-style flows.

```http
POST /api/realms/:realm/apps/:app/auth/otp/request    { "email": "u@acme.com" }
POST /api/realms/:realm/apps/:app/auth/otp/login      { "email": "u@acme.com", "code": "123456" }
```

`request` mails the user a 6-digit code valid for 10 minutes (configurable). `login` exchanges email + code for a fresh access/refresh pair. If the email doesn't exist yet, the user is auto-registered with no password — `users.has_password = false`. That's how you ship a 100% passwordless onboarding.

## TOTP (second factor)

Time-based one-time passwords via [`totp-rs`](https://docs.rs/totp-rs). Enrollment is a two-step dance so a misconfigured authenticator app can't accidentally lock the user out.

```http
POST /api/realms/:realm/apps/:app/auth/totp/enroll       [user token]
  → returns { "secret": "...", "qr_url": "otpauth://..." }

POST /api/realms/:realm/apps/:app/auth/totp/confirm      [user token]
  { "code": "123456" }
  → 200, TOTP is now active for this user

POST /api/realms/:realm/apps/:app/auth/totp/disable      [user token]
  { "code": "123456" }
```

After enrollment, the regular login becomes a two-step flow:

```http
POST /api/realms/:realm/apps/:app/auth/users/login
  { "email": "...", "password": "..." }
  → 202  { "mfa_required": true, "challenge_id": "..." }

POST /api/realms/:realm/apps/:app/auth/users/login/totp
  { "challenge_id": "...", "code": "123456" }
  → 200  { access_token, refresh_token, user }
```

App admins can force-unenroll a user with `DELETE /api/realms/:realm/apps/:app/users/:id/totp` (recovery flow).

## OAuth2 / OIDC

Built on the [`oauth2`](https://docs.rs/oauth2) crate. Providers are configured **per app** by an admin:

```http
PUT /api/realms/:realm/apps/:app/auth/oauth/providers/google
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
GET /api/realms/:realm/apps/:app/auth/oauth/google/authorize?redirect_uri=https://app/cb
  → 302 to Google with a state cookie set

POST /api/realms/:realm/apps/:app/auth/oauth/google/callback
  { "code": "...", "state": "...", "redirect_uri": "https://app/cb" }
  → 200  { access_token, refresh_token, user }
```

State is single-use and bound to the app. On first sign-in via OAuth, a user row is created with `has_password = false`. Subsequent logins match by `email` within the same app.

## Admin tokens

Master admins call `/_/auth/admin/login` with `{ username, password }`; realm admins call `/api/realms/:realm/auth/admin/login` with `{ email, password }`. Tokens carry a `role` claim (`master_admin`, `realm_admin`, `app_admin`, `user`) plus the scope claims that match it: master admin tokens have neither `realm` nor `app`; realm-admin tokens carry `realm`; app-admin tokens carry both `realm` and `app`; end-user tokens always carry both `realm` and `app` (users are per-app since the users-per-app refactor). Every protected handler enforces:

```rust
auth.require_master()?;             // master only
auth.require_realm_access(realm)?;  // master OR realm admin for this realm
auth.require_app_access(realm, app)?; // master OR realm admin OR app admin scoped here
```

Tokens are stateless. Revocation lives in an in-memory `DashSet` checked by middleware and auto-expires after the access-token TTL.

## TTL policies

`tokens.access_ttl_sec` and `tokens.refresh_ttl_sec` are [hierarchical policies](/concepts/hierarchical-policies). Default access TTL is 15 minutes, default refresh TTL is 30 days. Master sets the bounds; realms (and apps) tighten.
