# REST API reference

Every endpoint returns JSON (except file downloads). Errors return `{code, message}` with a standard HTTP status code; see [error codes](/reference/errors).

Unless noted, mutating routes require an `Authorization: Bearer <jwt>` header carrying a token of sufficient scope.

## Conventions

- `:realm`, `:app`, `:id`, `:field`, etc. are URL parameters.
- Pagination params are always `?page=N&per_page=M`. `per_page` caps at 200.
- Filter params are always `?filter=<expr>`. See the [filter syntax](/reference/filters).
- All timestamps are RFC 3339 (`2026-05-27T10:00:00Z`).

---

## Server

### Health

```http
GET /healthz
```

Returns `{ "initialized": bool }`. Always 200, even when the server is uninitialized.

### Setup wizard

```http
POST /_/setup
Content-Type: application/json

{ "password": "hunter22" }
```

RustBase auto-seeds a master admin row at first boot with `username = "admin"` and a NULL password. The setup wizard sets that password. Returns 201 on success, 409 if the password has already been set.

While the server is uninitialized, every other route returns **503 uninitialized** — that's the setup gate.

---

## Master admin auth

```http
POST /_/auth/admin/login
{ "username": "admin", "password": "hunter22" }

POST /_/auth/refresh
{ "refresh_token": "rfsh_..." }
```

Both return `{access_token, refresh_token, admin}`. The refresh token rotates on every exchange — old refresh tokens are revoked.

### JWKS

```http
GET /.well-known/jwks.json
GET /_/auth/jwks.json
```

Anonymous. Returns the JSON Web Key Set the server uses to sign
access tokens (`Content-Type: application/jwk-set+json`). Tokens are
signed with **RS256**; the `kid` header on each JWT matches a `kid`
in the JWKS. Cache for up to one hour (`Cache-Control: public,
max-age=3600`).

---

## Realm admin auth

```http
POST /api/realms/:realm/auth/admin/login
POST /api/realms/:realm/auth/refresh
```

Same shape as master, but scoped to a single realm.

```http
POST /api/realms/:realm/admins      [master only]
{ "email": "ops@acme.com", "password": "secretpw", "name": "Ops",
  "app_ids": [] }                   # empty = realm-wide; non-empty = app-scoped
```

Creates a realm or app admin. Only a master admin can call this.

---

## Realms

```http
GET    /api/realms
POST   /api/realms                          [master]
GET    /api/realms/:id
PATCH  /api/realms/:id                      [master]
DELETE /api/realms/:id                      [master, non-master only]
```

Body shape:

```json
{ "id": "acme", "name": "Acme Inc." }
```

`PATCH` accepts `{name}`. Deleting cascades: all apps, users, files, policies, audit entries under the realm vanish in one transaction.

---

## Apps

```http
GET    /api/realms/:realm/apps
POST   /api/realms/:realm/apps                      [realm admin]
GET    /api/realms/:realm/apps/:app
PATCH  /api/realms/:realm/apps/:app                 [realm admin]
DELETE /api/realms/:realm/apps/:app                 [realm admin]
```

Body shape mirrors realms. Creating an app initializes its `data.db` and picks up any JS/TS hooks already on disk under `data/hooks/<realm>/<app>/`.

---

## End-user auth

Self-service flows, **no admin token** required. End-users live per-app, so every URL carries an `/apps/:app/` segment:

```http
POST /api/realms/:realm/apps/:app/auth/users/register
{ "email": "u@acme.com", "password": "userpass1" }

POST /api/realms/:realm/apps/:app/auth/users/login
POST /api/realms/:realm/apps/:app/auth/users/refresh

POST /api/realms/:realm/apps/:app/auth/verify-email/request    [user token]
POST /api/realms/:realm/apps/:app/auth/verify-email/confirm    { "token": "..." }

POST /api/realms/:realm/apps/:app/auth/password-reset/request  { "email": "..." }
POST /api/realms/:realm/apps/:app/auth/password-reset/confirm  { "token": "...", "new_password": "..." }

POST /api/realms/:realm/apps/:app/auth/otp/request             { "email": "..." }
POST /api/realms/:realm/apps/:app/auth/otp/login               { "email": "...", "code": "123456" }

POST /api/realms/:realm/apps/:app/auth/totp/enroll             [user token]   → returns secret + QR url
POST /api/realms/:realm/apps/:app/auth/totp/confirm            [user token]   { "code": "123456" }
POST /api/realms/:realm/apps/:app/auth/totp/disable            [user token]   { "code": "123456" }
POST /api/realms/:realm/apps/:app/auth/users/login/totp        { "challenge_id": "...", "code": "123456" }
```

See the [authentication guide](/guide/authentication) for what each flow does.

---

## OAuth

End-user-facing:

```http
GET  /api/realms/:realm/apps/:app/auth/oauth/:provider/authorize?redirect_uri=...
POST /api/realms/:realm/apps/:app/auth/oauth/:provider/callback
{ "code": "...", "state": "...", "redirect_uri": "..." }
```

Admin-facing — manage which providers are wired up for this app:

```http
GET    /api/realms/:realm/apps/:app/auth/oauth/providers              [app admin]
GET    /api/realms/:realm/apps/:app/auth/oauth/providers/:provider
PUT    /api/realms/:realm/apps/:app/auth/oauth/providers/:provider
DELETE /api/realms/:realm/apps/:app/auth/oauth/providers/:provider
```

`PUT` body:

```json
{
  "client_id": "...",
  "client_secret": "...",          // optional on update; preserves existing ciphertext when omitted
  "config": {
    "auth_url": "...",
    "token_url": "...",
    "userinfo_url": "...",
    "scopes": ["openid", "email"]
  }
}
```

`client_secret` is encrypted at rest under the server's KEK. Admin reads **never** echo it back.

---

## Admin user management

```http
GET    /api/realms/:realm/apps/:app/users?page=&per_page=&q=          [app admin]
GET    /api/realms/:realm/apps/:app/users/:id
PATCH  /api/realms/:realm/apps/:app/users/:id/verify     { "verified": true }
DELETE /api/realms/:realm/apps/:app/users/:id/totp                    # force unenroll
DELETE /api/realms/:realm/apps/:app/users/:id
```

`q` is a substring match on email.

---

## Collections

```http
GET    /api/realms/:realm/apps/:app/collections
POST   /api/realms/:realm/apps/:app/collections        [app admin]
GET    /api/realms/:realm/apps/:app/collections/:name
PATCH  /api/realms/:realm/apps/:app/collections/:name  [app admin]
DELETE /api/realms/:realm/apps/:app/collections/:name  [app admin]
```

Body shape:

```json
{
  "schema": {
    "id": "posts",
    "kind": "base",          // "base" | "auth" | "view"
    "fields": [
      { "name": "title",  "kind": "text",    "required": true },
      { "name": "body",   "kind": "text" },
      { "name": "pinned", "kind": "bool" },
      { "name": "meta",   "kind": "json" },
      { "name": "author", "kind": "relation", "target": "users", "cascade_delete": true }
    ]
  }
}
```

Field kinds: `text`, `number`, `bool`, `json`, `datetime`, `email`, `url`, `select`, `relation`, `file`.

Creating an `auth` collection auto-adds the columns documented in the [collections guide](/guide/collections#auth-collections).

---

## Records

```http
GET    /api/realms/:realm/apps/:app/collections/:coll/records?page=&per_page=&filter=
POST   /api/realms/:realm/apps/:app/collections/:coll/records
GET    /api/realms/:realm/apps/:app/collections/:coll/records/:id
PATCH  /api/realms/:realm/apps/:app/collections/:coll/records/:id
DELETE /api/realms/:realm/apps/:app/collections/:coll/records/:id
```

Body is the record's field map (no nesting under `fields`):

```json
{ "title": "Hello", "pinned": true, "meta": {"tag": "intro"} }
```

Response:

```json
{
  "id": "01HXY...",
  "collection": "posts",
  "fields": { "title": "Hello", "pinned": true, "meta": {"tag":"intro"} },
  "created_at": "2026-05-27T10:00:00Z",
  "updated_at": "2026-05-27T10:00:00Z"
}
```

List response:

```json
{ "items": [...], "page": 1, "per_page": 30, "total_items": 42, "total_pages": 2 }
```

Access rules apply per `(collection, action)` pair — see [collections](/guide/collections#access-rules).

---

## Access rules

```http
GET    /api/realms/:realm/apps/:app/collections/:coll/access_rules
PUT    /api/realms/:realm/apps/:app/collections/:coll/access_rules/:action
DELETE /api/realms/:realm/apps/:app/collections/:coll/access_rules/:action
```

`action` is one of `list`, `get`, `create`, `update`, `delete`.

Body of `PUT`:

```json
{ "template": "any" }
{ "template": "auth" }
{ "template": "admin" }
{ "template": "filter", "filter": "@request.auth.id != \"\" && owner = @request.auth.id" }
```

`@request.auth` exposes the current user (or admin) inside the filter.

---

## Files

```http
GET    /api/realms/:realm/apps/:app/files
POST   /api/realms/:realm/apps/:app/files
GET    /api/realms/:realm/apps/:app/files/:id
GET    /api/realms/:realm/apps/:app/files/:id/meta
DELETE /api/realms/:realm/apps/:app/files/:id
```

Upload:

```http
POST /api/realms/:realm/apps/:app/files
Authorization: Bearer <token>
X-Filename: kitten.png
Content-Type: image/png

<raw bytes>
```

Response: `{id, filename, mime, size, created_at}`.

Download returns the raw bytes with the stored `Content-Type` and an `X-Filename` header echoing the saved filename.

---

## Realtime

Server-Sent Events:

```http
GET /api/realms/:realm/apps/:app/collections/:coll/events
Authorization: Bearer <token>
```

Event types: `record_created`, `record_updated`, `record_deleted`. Each carries the record JSON.

---

## Custom JS routes

JS hooks register their own endpoints via `routerAdd`. Mount point:

```http
ANY /api/realms/:realm/apps/:app/custom/*path
```

The matcher delegates to the JS shim's `$app.routerAdd` table; missing handlers return 404. See [hooks](/guide/hooks#custom-http-routes).

---

## Policies

```http
GET    /api/system/policies                                 [master]
GET    /api/system/policies/:field
PUT    /api/system/policies/:field                          # body: PolicySpec
DELETE /api/system/policies/:field

GET    /api/realms/:realm/policies                          [realm admin]
GET    /api/realms/:realm/policies/:field
PUT    /api/realms/:realm/policies/:field
DELETE /api/realms/:realm/policies/:field

GET    /api/realms/:realm/apps/:app/policies                [app admin]
GET    /api/realms/:realm/apps/:app/policies/:field
PUT    /api/realms/:realm/apps/:app/policies/:field
DELETE /api/realms/:realm/apps/:app/policies/:field
```

Body of `PUT` is a `PolicySpec`:

```json
{ "kind": "range",    "min": 6, "max": 64 }
{ "kind": "toggle",   "state": "locked", "value": true }
{ "kind": "toggle",   "state": "open",   "default": false }
{ "kind": "enum_set", "allowed": ["google", "github"] }
{ "kind": "free" }
```

Response of `PUT` includes a `cascaded` array describing any child values that were auto-clamped — see [hierarchical policies](/concepts/hierarchical-policies).

---

## Audit log

```http
GET /api/system/audit?page=&per_page=&action=&actor=                   [master]
GET /api/realms/:realm/audit?page=&per_page=&action=&actor=            [realm admin]
GET /api/realms/:realm/apps/:app/audit?page=&per_page=&action=&actor=  [app admin]
```

Each entry: `{id, ts, actor, action, target, details}`. `action` is a case-insensitive substring match; `actor` is exact.

---

## Hook source files

```http
GET    /api/realms/:realm/apps/:app/hooks                    [app admin]
GET    /api/realms/:realm/apps/:app/hooks/:filename
PUT    /api/realms/:realm/apps/:app/hooks/:filename          # { source: "..." }
DELETE /api/realms/:realm/apps/:app/hooks/:filename
POST   /api/realms/:realm/apps/:app/hooks/reload
```

`PUT` and `DELETE` automatically trigger a reload and return `{file, reload: {loaded, errors}}` (or just `{loaded, errors}` for `DELETE` and `/reload`). Compile errors live in the `errors` array so the dashboard can surface them inline.

Filename validation: must end in `.js` or `.ts`, must not contain `/`, `\`, or `..`. First character must be a letter, digit, or `_`.
