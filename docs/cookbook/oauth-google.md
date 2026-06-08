# OAuth login (Google)

Wire Google's OAuth 2.0 sign-in into a RustBase workspace. The recipe uses Google as the example; the same shape works for GitHub, Microsoft, or any OIDC-compliant provider.

## What you'll build

A "Sign in with Google" button on your client app that, on click:

1. Bounces the user to Google's consent screen.
2. Returns to your client with a code.
3. Your client forwards the code to RustBase.
4. RustBase exchanges it for the user's email, links it to a workspace user, and issues an access + refresh token bound to that user.

## 1. Register a Google OAuth client

In the [Google Cloud Console](https://console.cloud.google.com/) under **APIs & Services → Credentials**:

- Application type: **Web application**.
- Authorised JavaScript origins: your client's origin (`https://app.example.com`).
- Authorised redirect URIs: `https://<your-rustbase-host>/_/auth/oauth/google/callback`.

Save the **Client ID** and **Client secret**. You'll paste them into the dashboard in a moment.

## 2. Add the provider in the dashboard

Visit `/_/workspaces/acme/oauth/new`. The dashboard ships **Google / GitHub / Microsoft presets** that prefill every URL field; pick Google.

| Field | Value |
|---|---|
| Provider slug | `google` |
| Client ID | from step 1 |
| Client secret | from step 1 |
| Scopes | `openid email profile` |
| Auth URL | `https://accounts.google.com/o/oauth2/v2/auth` |
| Token URL | `https://oauth2.googleapis.com/token` |
| Userinfo URL | `https://openidconnect.googleapis.com/v1/userinfo` |
| Userinfo ID field | `/sub` |
| Userinfo email field | `/email` |

The client secret is encrypted at rest under the workspace's KEK (a 256-bit key stored in `system.db._secrets`). The plaintext never round-trips back to the dashboard.

## 3. Build the consent-screen URL on your client

```ts
const params = new URLSearchParams({
  client_id:     '<google client id>',
  redirect_uri:  'https://<your-rustbase-host>/_/auth/oauth/google/callback',
  response_type: 'code',
  scope:         'openid email profile',
  access_type:   'offline',
  state:         crypto.randomUUID(),
});
window.location = `https://accounts.google.com/o/oauth2/v2/auth?${params}`;
```

Persist `state` to `sessionStorage` before redirecting; you'll match it on the way back to defeat CSRF.

## 4. Receive the callback

Google redirects to `https://<rustbase>/_/auth/oauth/google/callback?code=…&state=…`. The dashboard's callback page:

1. Validates `state` against the value you persisted.
2. Forwards the code to RustBase:

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/oauth/google/exchange \
  -H "content-type: application/json" \
  -d "{\"code\":\"$CODE\",\"redirect_uri\":\"https://<rustbase>/_/auth/oauth/google/callback\"}"
```

The server:
- Posts to Google's token endpoint with the encrypted secret.
- Pulls `userinfo` and reads the `email` + `sub` per the JSON pointers configured above.
- Finds-or-creates a workspace user keyed by email.
- Records the linkage `(provider=google, provider_user_id=<sub>)` in `oauth_links`.
- Returns the standard auth response: `{ access_token, refresh_token, user }`.

## PKCE

Public clients — single-page apps, native mobile — should add PKCE. RustBase accepts the `code_verifier` on the exchange call:

```sh
curl -s http://localhost:8080/api/workspaces/acme/auth/oauth/google/exchange \
  -H "content-type: application/json" \
  -d "{
        \"code\":          \"$CODE\",
        \"redirect_uri\":  \"…\",
        \"code_verifier\": \"$VERIFIER\"
      }"
```

…and the redirect URL on the way out includes:

```text
&code_challenge=<S256(verifier)>
&code_challenge_method=S256
```

Generate the verifier (43–128 unreserved chars) on the client, store it in `sessionStorage`, and send it on the exchange. The server forwards it to Google verbatim.

## Variations

- **GitHub** — pick the GitHub preset. Note that GitHub's userinfo response uses `/id` (an integer) as the stable identifier and `/email` may be empty (the user can mark their email private). Add the `user:email` scope and the server will fall back to the `/user/emails` endpoint to find a verified address.
- **Generic OIDC** — any provider that exposes `.well-known/openid-configuration` works. Pull the URLs from there and paste them into the dashboard.
- **Restrict to one domain** — add an `email_domain_allowlist` to the provider config (planned for a future release), or write a `onUserBeforeRegister` hook that rejects emails outside `@example.com`.

## Gotchas

- **The redirect URI must match exactly** — `https://app.example.com/callback` is NOT the same as `https://app.example.com/callback/`. Google's error in this case is a generic "redirect URI mismatch" with no further detail.
- **Existing email + new OAuth link** — RustBase finds-by-email first. A user who registered with a password gets the OAuth link added to their existing row; the password still works alongside.
- **OAuth tokens are NOT stored.** RustBase reads `userinfo` once at exchange time and discards Google's access token. To call Google APIs on behalf of the user later, you need to add a hook that keeps Google's token in your own table.
