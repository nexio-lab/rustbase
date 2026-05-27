# Collections & records

A collection is a typed table inside an app. It has an `id` (the table name), a `kind`, and a list of fields.

## Kinds

| Kind | Use |
|---|---|
| `base` | Plain records. The default. |
| `auth` | End-users. Auto-adds `email`, `password_hash`, `verified`, `last_login`, `oauth_providers` fields. |
| `view` | SQL-backed, read-only. *(Coming soon.)* |

You can have many `base` collections per app. Every app already gets a built-in `users` table (in its `data.db`) that the `/auth/users/*` endpoints write to; an `auth` collection is an alternative when you need extra user fields beyond the built-in shape.

## Field types

```json
{ "name": "title", "kind": "text",     "required": true,  "max": 200 }
{ "name": "age",   "kind": "number",   "min": 0, "max": 130 }
{ "name": "tags",  "kind": "json" }
{ "name": "born",  "kind": "datetime" }
{ "name": "ok",    "kind": "bool",     "default": false }
{ "name": "email", "kind": "email" }
{ "name": "site",  "kind": "url" }
{ "name": "role",  "kind": "select",   "options": ["admin","user","guest"] }
{ "name": "author","kind": "relation", "target": "users", "cascade_delete": true }
{ "name": "cover", "kind": "file" }
```

Every field has `required`, `default`, and (for strings/numbers) bounds. Validation runs on every write — invalid payloads return 400.

## Create a collection

```http
POST /api/realms/:realm/apps/:app/collections
{
  "schema": {
    "id": "posts",
    "kind": "base",
    "fields": [
      { "name": "title", "kind": "text", "required": true },
      { "name": "body",  "kind": "text" },
      { "name": "pinned","kind": "bool", "default": false }
    ]
  }
}
```

Behind the scenes RustBaas issues a `CREATE TABLE` and an `INSERT` into `_collections`. Reserved table names (`_collections`, `_access_rules`, `_files`, `_email_verifications`, `_password_resets`, `_email_otps`, `_oauth_states`, `_oauth_links`, `_oauth_providers`, `users`, `master_admins`, `realm_admins`, `audit_log`, `policies`) are rejected.

## Patch a schema

```http
PATCH /api/realms/:realm/apps/:app/collections/posts
{
  "schema": {
    "id": "posts",
    "kind": "base",
    "fields": [
      { "name": "title",   "kind": "text", "required": true },
      { "name": "body",    "kind": "text" },
      { "name": "summary", "kind": "text" }
    ]
  }
}
```

Adding a field issues `ALTER TABLE ADD COLUMN`. Removing a field is destructive and requires the value `?confirm=drop` on the URL. Renaming is not supported — drop then re-add.

## Records

### Create

```http
POST /api/realms/:realm/apps/:app/collections/posts/records
{ "title": "Hello", "body": "first" }
```

Response:

```json
{
  "id": "01HXY...",
  "collection": "posts",
  "fields": { "title": "Hello", "body": "first" },
  "created_at": "2026-05-27T10:00:00Z",
  "updated_at": "2026-05-27T10:00:00Z"
}
```

IDs are v7 UUIDs — lexicographically sortable by creation time.

### Read

```http
GET /api/realms/:realm/apps/:app/collections/posts/records/:id
```

### Update

```http
PATCH /api/realms/:realm/apps/:app/collections/posts/records/:id
{ "body": "edited" }                # partial; omitted fields stay put
```

### Delete

```http
DELETE /api/realms/:realm/apps/:app/collections/posts/records/:id
```

### List + filter + paginate

```http
GET /api/realms/:realm/apps/:app/collections/posts/records?page=1&per_page=30&filter=pinned = true
```

Response:

```json
{
  "items":       [ ... ],
  "page":        1,
  "per_page":    30,
  "total_items": 7,
  "total_pages": 1
}
```

See the [filter syntax reference](/reference/filters).

## Auth collections

Creating a collection with `kind: "auth"` automatically adds:

| Field | Type | Notes |
|---|---|---|
| `email` | email | unique |
| `password_hash` | text | argon2; never returned in responses |
| `verified` | bool | flipped by the verify-email flow |
| `last_login` | datetime | updated on every successful login |
| `oauth_providers` | json | array of linked provider records |

Each app has its own `users` table — two apps in the same realm can share an email address but the rows are independent identities. If you want one identity that spans products, run them as a single app.

## Access rules

Every collection has five actions (`list`, `get`, `create`, `update`, `delete`), each governed by an access rule.

```http
GET    /api/realms/:realm/apps/:app/collections/:coll/access_rules
PUT    /api/realms/:realm/apps/:app/collections/:coll/access_rules/:action
DELETE /api/realms/:realm/apps/:app/collections/:coll/access_rules/:action
```

Rule templates:

| Template | Who passes |
|---|---|
| `any` | anyone, including unauthenticated |
| `auth` | any signed-in user |
| `admin` | admins only (default) |
| `filter` | the request passes when the supplied [filter](/reference/filters) evaluates true |

Example filter rule — "only the post's author can update":

```json
{ "template": "filter", "filter": "@request.auth.id != \"\" && author = @request.auth.id" }
```

`@request.auth.id`, `@request.auth.email`, `@request.auth.role`, `@request.body.<field>` are all available inside an access-rule filter.

## Lifecycle

Every successful record write fires lifecycle events the runtime can subscribe to:

- `onRecordBeforeCreate(coll, fn)` — return `false` or `throw` to veto.
- `onRecordAfterCreate(coll, fn)`
- `onRecordBeforeUpdate(coll, fn)`
- `onRecordAfterUpdate(coll, fn)`
- `onRecordBeforeDelete(coll, fn)`
- `onRecordAfterDelete(coll, fn)`

The same events publish to the [realtime broker](/guide/realtime) for SSE/WebSocket subscribers. See [hooks](/guide/hooks).
