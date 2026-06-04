# Files

Every app has its own file store. Binary data goes to disk (or S3); a metadata row lives in the app's `data.db`.

## Upload

Send the bytes raw — no `multipart/form-data` parsing. The `X-Filename` header carries the saved name; `Content-Type` carries the MIME.

```http
POST /api/workspaces/:workspace/apps/:app/files
Authorization: Bearer <token>
X-Filename: kitten.png
Content-Type: image/png

<raw bytes>
```

```sh
curl -X POST http://localhost:8080/api/workspaces/acme/apps/web/files \
  -H "authorization: Bearer $TOKEN" \
  -H "x-filename: kitten.png" \
  -H "content-type: image/png" \
  --data-binary @kitten.png
```

Response (201):

```json
{
  "id":         "01HXY...",
  "filename":   "kitten.png",
  "mime":       "image/png",
  "size":       17,
  "created_at": "2026-05-27T10:00:00Z"
}
```

The dashboard's **Files** tab is a drag-and-drop frontend over the same endpoint.

## Limits

A single upload caps at `MAX_UPLOAD_BYTES` (10 MB default). Override via the `storage.max_upload_mb` policy — master sets a ceiling, workspaces tighten, apps pick.

## Download

```http
GET /api/workspaces/:workspace/apps/:app/files/:id
Authorization: Bearer <token>
```

Returns the raw bytes. `Content-Type` is the stored MIME; `X-Filename` echoes the saved name so clients can save with the original filename.

> **Browser tip:** the dashboard uses `fetch` + `Blob` + `URL.createObjectURL` to download, because plain `<a href download>` can't carry the Bearer header.

## Metadata only

```http
GET /api/workspaces/:workspace/apps/:app/files/:id/meta
```

Returns the same JSON as the upload response — no binary fetch.

## List + delete

```http
GET    /api/workspaces/:workspace/apps/:app/files
DELETE /api/workspaces/:workspace/apps/:app/files/:id
```

## Linking from records

A field of `kind: "file"` stores the file id (a uuid). Render a thumbnail by following the link to `/files/:id`:

```ts
const post = $app.records.findOne("posts", id);
const coverUrl = `/api/workspaces/acme/apps/web/files/${post.fields.cover}`;
```

Files referenced by a record are not auto-deleted when the record dies — collect-on-delete is intentionally opt-in (use a hook).

## Backends

Configured globally in `rustbase.toml`:

```toml
# Default: local disk.
[storage]
backend = "local"

# Or any S3-compatible bucket.
[storage.s3]
bucket            = "rustbase-prod"
region            = "us-east-1"
endpoint          = "https://s3.example.com"   # omit for AWS
access_key_id     = "AKIA..."
secret_access_key = "..."
virtual_hosted_style_request = false           # false for MinIO, true for AWS
```

Switching backends doesn't touch the metadata in `data.db` — the keys are stable. To migrate existing files, copy them between backends with the matching key prefix; everything else keeps working.

[Cloudflare R2](https://developers.cloudflare.com/r2/) and [MinIO](https://min.io) both work — set `endpoint` to point at them. AWS S3 needs no `endpoint`.

## What's on disk

With the local backend:

```
data/workspaces/<workspace>/apps/<app>/storage/
  └── <file_id>           # the raw bytes, no extension
```

The id is the file's REST path tail. No subdirectories, no sharding — SQLite stays the index, the filesystem stays a dumb blob store.
