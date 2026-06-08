# Add a custom HTTP route

How to drop a `.js` (or `.ts`) file into `data/hooks/<workspace>/<app>/` and have it expose a new endpoint on the app's REST surface. The hook runs inside the sandboxed QuickJS runtime — no Node.js needed.

## What you'll build

`POST /api/workspaces/acme/apps/mobile/custom/notes/import-csv` that takes a CSV body, parses it server-side, and inserts a record per row via `$app.records`. The route is gated by the same auth middleware as the rest of the API.

## 1. Drop the hook file

```ts
// data/hooks/acme/mobile/import-csv.ts
routerAdd('POST', '/custom/notes/import-csv', (c) => {
  // c.request — the inbound request: method, headers, body, auth.
  // c.response — the response builder: status(), header(), json(), text().

  if (!c.request.auth || c.request.auth.role !== 'app_admin') {
    return c.response.status(403).json({ message: 'admin only' });
  }

  const body = c.request.body();           // string for text/*, ArrayBuffer for binary.
  const rows = body.split('\n').filter(Boolean);
  const created: string[] = [];

  for (const line of rows) {
    const [title, pinnedRaw] = line.split(',');
    const rec = $app.records.create('notes', {
      title:  title.trim(),
      pinned: pinnedRaw?.trim() === 'true',
    });
    created.push(rec.id);
  }

  return c.response.json({ created_count: created.length, ids: created });
});
```

TypeScript files are transpiled at load time via `swc`; you can ship `.js` directly to skip that step.

## 2. Reload hooks

Hooks are loaded **once at boot** for every `(workspace, app)` directory. After dropping the file, restart the server (or hit `POST /api/workspaces/acme/apps/mobile/_hooks/reload` if you've enabled the dashboard's hot-reload toggle).

```sh
# stop
pkill rustbase

# start
./rustbase
```

Server log:

```
INFO rustbase_runtime: loaded JS hooks workspace=acme app=mobile files=1
INFO rustbase_runtime: routerAdd POST /custom/notes/import-csv
```

## 3. Call it

```sh
curl -s http://localhost:8080/api/workspaces/acme/apps/mobile/custom/notes/import-csv \
  -H "authorization: Bearer $AT" \
  -H "content-type: text/csv" \
  --data-binary $'first note,true\nsecond note,false\nthird note,true'
```

```json
{ "created_count": 3, "ids": ["01HXY…","01HXY…","01HXY…"] }
```

## The `$app` global

Hooks see a sandboxed global called `$app` with the in-process surface:

- `$app.records` — `create`, `update`, `delete`, `findRecord(coll, id)`, `findRecordsByFilter(coll, filter)`. Same `FilterNode` AST as the REST API.
- `$app.realtime.publish(coll, record)` — push to subscribers; respects collection access rules.
- `$app.audit.log(action, scope, payload)` — drop a structured audit entry.
- `$app.mailer.send({to, subject, body})` — emit an email via the configured mailer (`LogMailer` in dev, `SmtpMailer` otherwise).
- `$app.fetch(url, init)` — outbound HTTP. **Off by default**. Allowed hosts come from the `[hooks.fetch] allowed_hosts` config — every other URL throws `Forbidden` before any IO.

## Sandbox limits

Each hook invocation runs under a per-app sandbox bounded by:

| Limit | Default | Config key |
|---|---|---|
| CPU time | 200 ms | hierarchical policy `hook.cpu_ms` |
| Memory | 64 MiB | `hook.memory_mb` |
| Network | none | `hook.fetch.allowed_hosts` |
| Filesystem | none | always off |

The limits are bounded by the workspace, which is bounded by the master config — so a master admin can hard-cap what any app admin can lift on their own app.

## Variations

- **Validation hook** — register `onRecordBeforeCreate('notes', (c) => { … })` to reject writes early. Returning `c.error('validation', 'message')` becomes a 400 on the REST side.
- **Wire-format webhook** — `$app.fetch('https://hooks.slack.com/…', { method: 'POST', body: JSON.stringify(payload) })` after a record write. Don't forget to add the host to the fetch allowlist.
- **Scheduled job** — `cronAdd('hourly', '0 * * * *', () => { … })` runs on the in-process scheduler. Same sandbox limits apply.

## Gotchas

- **`$app.fetch` calls run sync** from the hook's point of view, but the host bridge dispatches them on a background tokio runtime. They DO count toward the CPU limit; a hung HTTP call eventually trips the deadline and the hook returns `error: cpu_exceeded`.
- **No `require` / `import`.** Every hook file is a single self-contained script. The router and event hooks are all registered through globals (`routerAdd`, `onRecordBeforeCreate`, `cronAdd`).
