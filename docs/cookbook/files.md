# Upload and attach a file

A multipart-upload recipe that puts a binary on the app's storage backend, attaches it to a record via a `file` field, and serves it back to the client by ID.

## What you'll build

A `notes` collection with a `cover` file field. The client uploads the image, the server stores it via the configured `object_store` backend (local disk or S3), and a subsequent `GET` on the record returns the cover URL.

## 1. Add the file field to the schema

```sh
curl -s -X PATCH http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes \
  -H "authorization: Bearer $AT" \
  -H "content-type: application/json" \
  -d '{
    "schema": {
      "id":     "notes",
      "kind":   "base",
      "fields": [
        { "name": "title", "kind": "text", "required": true },
        { "name": "cover", "kind": "file", "max_size": 5242880,
          "mime_types": ["image/jpeg","image/png","image/webp"] }
      ]
    }
  }'
```

`max_size` is bytes (`5 MiB` here). `mime_types` is checked against the request's `Content-Type` part header at upload time; anything outside the allowlist is rejected with `415 Unsupported Media Type`.

## 2. Upload

The file endpoint is **separate from the records endpoint** — uploads always return a file ID first, which the client then references when creating or updating a record.

```sh
FILE_ID=$(curl -s http://localhost:8080/api/workspaces/acme/apps/mobile/files \
  -H "authorization: Bearer $AT" \
  -F file=@./cover.jpg \
  | jq -r .id)

echo "Uploaded as $FILE_ID"
```

Behind the scenes:
- The binary is streamed to the storage backend (local → `data/workspaces/acme/apps/mobile/storage/<id>`; S3 → `s3://<bucket>/<prefix>/<id>`).
- A row in the app's `_files` table tracks `(id, mime, size, sha256, uploaded_at, uploaded_by)`.
- The binary **never** lives in the SQLite database.

## 3. Attach to a record

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/records \
  -H "authorization: Bearer $AT" \
  -H "content-type: application/json" \
  -d "{\"title\":\"hello\",\"cover\":\"$FILE_ID\"}"
```

The server confirms that the file ID exists, that the caller uploaded it, and that the mime type matches the field's allowlist. Reassigning an existing record's `cover` to a different file is fine — orphaned files (no record references) are garbage-collected by the periodic GC job (default: every 6 hours).

## 4. Read it back

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/records/$RECORD_ID \
  -H "authorization: Bearer $AT"
```

```json
{
  "id":     "01HXY…",
  "fields": {
    "title": "hello",
    "cover": {
      "id":   "01HZA…",
      "url":  "/api/workspaces/acme/apps/mobile/files/01HZA…/serve",
      "mime": "image/jpeg",
      "size": 142817
    }
  }
}
```

The `url` is the public-facing GET endpoint. Auth rules on the record apply transitively — a user who can't read the row can't fetch the file.

## 5. Switch to S3 in production

Add `[storage.s3]` to `rustbase.toml`:

```toml
[storage.s3]
bucket            = "rustbase-prod"
region            = "us-east-1"
access_key_id     = "AKIA…"
secret_access_key = "…"
# Optional — set for non-AWS S3-compatible (R2, MinIO, Backblaze B2):
# endpoint = "https://<account>.r2.cloudflarestorage.com"
```

Existing on-disk paths under `data/workspaces/.../storage/` are **not** migrated — they keep returning local files. The storage backend is per-server, not per-record, so a switch only affects new uploads. A bulk-copy + delete step is needed to move old files; see [Backups](/guide/backups) for that pattern.

## Variations

- **Public file** — set `access_rule = "@authenticated || public = true"` on the collection and stash a `public` boolean field. The serve endpoint then waives auth for rows marked public.
- **Avatar field on a user collection** — use `kind: "auth"` on the collection and add a `cover`-style field; the auto-generated `email`/`password_hash` columns stay alongside.
- **Multiple files per record** — drop the field on the schema, store a JSON array of file IDs, and resolve them client-side. RustBase's `kind: "file"` is single-file by design.

## Gotchas

- The body cap (`[http] max_body_bytes`, default 8 MiB) applies to **every** request including multipart uploads. Bump it before `max_size` on a field. Without that, a 5 MiB file with multipart envelope overhead can hit the entry-layer limit before reaching your handler.
- The default file GC interval (6h) means orphaned files linger for a while. Manual cleanup: `DELETE FROM _files WHERE id NOT IN (SELECT … FROM <collections that reference files>)` — but please don't write that by hand, the dashboard's storage view exposes a "Force GC" button.
