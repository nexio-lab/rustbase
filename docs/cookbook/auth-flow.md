# End-to-end sign-up flow

A drop-in walkthrough for the user-facing side of authentication: register, verify the email, log in, refresh, and revoke. Every snippet is a real `curl` call against an app named `mobile` in workspace `acme`.

## What you'll build

A working sign-up flow that ends with the client holding a short-lived **access token** + an opaque **refresh token**, both bound to `(workspace_id, user_id)` so the same session works against every app in the workspace.

## 1. Register

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/users/register \
  -H "content-type: application/json" \
  -d '{"email":"alice@example.com","password":"hunter22"}'
```

```json
{"id":"01HXY…","email":"alice@example.com","verified":false}
```

The server hashes the password with `argon2`, persists the row in the workspace's `workspace.db`, and emits a `user.before_create` → `user.after_create` hook pair if any are registered. The user is **not** yet verified — the next step.

## 2. Send the verification email

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/verification/request \
  -H "content-type: application/json" \
  -d '{"email":"alice@example.com"}'
```

The mailer dispatches a 6-digit OTP. In dev — when no `[mail.smtp]` is configured — the `LogMailer` writes the code to the server's stdout: `verification OTP code=314159`. Bring up MailHog from `infra/docker-compose.yml` to read it from a web UI instead.

## 3. Confirm

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/verification/confirm \
  -H "content-type: application/json" \
  -d '{"email":"alice@example.com","code":"314159"}'
```

The server flips `verified=true` and removes the OTP from `_verifications`. Codes expire after 15 minutes by default.

## 4. Log in

```sh
RESP=$(curl -s http://localhost:8080/api/workspaces/acme/auth/users/login \
  -H "content-type: application/json" \
  -d '{"email":"alice@example.com","password":"hunter22"}')

AT=$(echo "$RESP" | jq -r .access_token)
RT=$(echo "$RESP" | jq -r .refresh_token)
```

Response shape:

```json
{
  "access_token":  "eyJ…",       // RS256 JWT, 15-minute TTL
  "refresh_token": "rfsh_…",     // opaque, 30-day TTL
  "user":          { "id": "…", "email": "alice@example.com", "verified": true }
}
```

## 5. Call an authenticated endpoint

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/records \
  -H "authorization: Bearer $AT" \
  -H "content-type: application/json" \
  -d '{"title":"hello"}'
```

The middleware decodes the JWT, checks `(workspace_id, user_id)` against the in-memory revocation set, then dispatches to the records handler.

## 6. Refresh before the access token expires

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/refresh \
  -H "content-type: application/json" \
  -d "{\"refresh_token\":\"$RT\"}"
```

The server rotates the refresh token — the old one becomes invalid, a new one is issued. Persist the new value before discarding the old one, otherwise a crash mid-flight leaves the client without a way back in.

## 7. Revoke (log out)

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/logout \
  -H "authorization: Bearer $AT"
```

The user's `(workspace_id, user_id)` lands in the in-memory revocation set; outstanding access tokens are rejected until they naturally expire (worst case: 15 minutes). The refresh token is also deleted so it can't be exchanged again.

## Browser clients: use HttpOnly cookies

The setup wizard, dashboard login, and the dashboard auth-refresh path all use **HttpOnly cookies** (`rb_at`, `rb_rt`) instead of putting the tokens in JS. The same shape works for your own browser apps — point your sign-up form at:

```http
POST /api/workspaces/acme/auth/users/login
Content-Type: application/json
Accept: application/json

{ "email": "...", "password": "..." }
```

…and add the header `X-Rb-Cookie-Auth: 1` to opt the response into cookie-mode. The body still includes the user object so your client can render the UI; the tokens themselves never touch JS.

## Variations

- **Magic-link login** — skip the password step entirely. POST to `/auth/otp/request`, prompt for the code, POST to `/auth/otp/confirm`. Same response shape.
- **TOTP enrolment** — once logged in, POST to `/auth/totp/enroll` to receive a secret + provisioning URI; future logins return a `requires_totp: true` body that your client handles by re-posting with `{ "totp": "123456" }`.
- **Per-app admins** — replace `/auth/users/login` with `/auth/admin/login` for workspace admins, or `/apps/:app/auth/admin/login` for app-scoped admins. Same token shape.
