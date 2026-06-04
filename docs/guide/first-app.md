# First app, end-to-end

This walkthrough builds a tiny note-taking backend entirely through `curl`. By the end you'll have a workspace, an app, a schema, a registered user, an authenticated record, and a realtime subscription — all without touching the dashboard.

## Sign in as master admin

Assuming you ran the setup wizard and set the password to `hunter22` (the master admin username is fixed at boot to `admin`):

```sh
TOKEN=$(curl -s http://localhost:8080/_/auth/admin/login \
  -H "content-type: application/json" \
  -d '{"username":"admin","password":"hunter22"}' \
  | jq -r .access_token)
```

## Create a workspace

```sh
curl -s http://localhost:8080/api/workspaces \
  -H "authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{"id":"acme","name":"Acme Inc."}' | jq
```

## Create an app

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps \
  -H "authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{"id":"notes","name":"Notes"}' | jq
```

## Define a collection

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/notes/collections \
  -H "authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{
    "schema": {
      "id": "posts",
      "kind": "base",
      "fields": [
        {"name": "title",  "kind": "text", "required": true},
        {"name": "body",   "kind": "text"},
        {"name": "pinned", "kind": "bool"}
      ]
    }
  }' | jq
```

## Register and log in as an end-user

End-users live per-app: register against the specific `(workspace, app)` you want to authenticate into.

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/notes/auth/users/register \
  -H "content-type: application/json" \
  -d '{"email":"user@acme.com","password":"userpass1"}'

UTOKEN=$(curl -s http://localhost:8080/api/workspaces/acme/apps/notes/auth/users/login \
  -H "content-type: application/json" \
  -d '{"email":"user@acme.com","password":"userpass1"}' \
  | jq -r .access_token)
```

## Create a record

By default, access rules require an admin token. For an open app, override the rules through the dashboard (or `PUT .../access_rules/create` with `{"template":"any"}`). For this walkthrough we'll keep using `$TOKEN`:

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/notes/collections/posts/records \
  -H "authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{"title":"Hello","body":"first post","pinned":true}' | jq
```

## Query with a filter

```sh
curl -s -G http://localhost:8080/api/workspaces/acme/apps/notes/collections/posts/records \
  -H "authorization: Bearer $TOKEN" \
  --data-urlencode 'filter=pinned = true && title ~ "Hello"' | jq
```

See the [filter syntax reference](/reference/filters) for the full grammar.

## Subscribe to realtime updates

```sh
curl -N -H "authorization: Bearer $TOKEN" \
  http://localhost:8080/api/workspaces/acme/apps/notes/collections/posts/events
```

Leave that open in one terminal, run the create-record `curl` from another, and you'll see the `record_created` event stream in.

## Upload a file

```sh
curl -s -X POST http://localhost:8080/api/workspaces/acme/apps/notes/files \
  -H "authorization: Bearer $TOKEN" \
  -H "x-filename: kitten.png" \
  -H "content-type: image/png" \
  --data-binary @kitten.png | jq
```

## Drop in a hook

```sh
mkdir -p data/hooks/acme/notes
cat > data/hooks/acme/notes/log.ts <<'EOF'
$app.onRecordAfterCreate("posts", (rec) => {
  $app.log(`new post: ${rec.fields.title}`);
});
EOF
```

Reload the app's hooks through the dashboard (Hooks → Reload) or restart the server. Now every `record_created` event prints a line into the server log.

## Next steps

You've just touched every layer of RustBase. From here:

- **Tune access rules** — [collections & records](/guide/collections#access-rules)
- **Add OAuth or TOTP** — [authentication](/guide/authentication)
- **Schedule cron jobs from JS** — [hooks](/guide/hooks#cron)
- **Promote a backup target** — [Litestream](/guide/backups)
