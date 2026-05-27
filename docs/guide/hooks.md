# Hooks (JS/TS)

RustBaas embeds [`rquickjs`](https://github.com/DelSkayn/rquickjs) — a QuickJS Rust binding — to run JavaScript and (transpiled) TypeScript hook files at runtime. No Node.js is required; the QuickJS engine ships with the binary.

Drop a file into `data/hooks/<realm>/<app>/` and a hook lights up the next time the app's runtime loads (which happens on app creation, on server boot, and on every dashboard **Reload** or write through the [hooks REST endpoint](/reference/rest-api#hook-source-files)).

## Hello, hook

```ts
// data/hooks/acme/notes/log.ts
$app.onRecordAfterCreate("posts", (rec) => {
  $app.log(`new post: ${rec.fields.title} (id=${rec.id})`);
});
```

`$app` is the global the runtime injects. Everything you can do from a hook hangs off it.

## Record lifecycle

| Hook | Fires |
|---|---|
| `$app.onRecordBeforeCreate(coll, fn)` | Right before the row hits the DB. Return `false` (or throw) to veto. |
| `$app.onRecordAfterCreate(coll, fn)` | Immediately after a successful insert. |
| `$app.onRecordBeforeUpdate(coll, fn)` | Before the `UPDATE`. Receives `(existing, patch)`. |
| `$app.onRecordAfterUpdate(coll, fn)` | After a successful update. |
| `$app.onRecordBeforeDelete(coll, fn)` | Before the `DELETE`. |
| `$app.onRecordAfterDelete(coll, fn)` | After a successful delete. |

`rec` is a record object: `{ id, collection, fields, created_at, updated_at }`. Mutations to `rec.fields` inside a `Before` hook are persisted.

## User lifecycle

| Hook | Fires |
|---|---|
| `$app.onUserBeforeLogin(fn)` | After password check, before token issuance. Throw to veto. |
| `$app.onUserAfterLogin(fn)` | After a successful login. |
| `$app.onUserAfterRegister(fn)` | After `POST /auth/users/register`. |

User-lifecycle hooks are realm-wide — every app's hooks for the same realm fire on every login.

## Custom HTTP routes

Add your own endpoint without touching Rust:

```ts
$app.routerAdd("GET", "/hello", (ctx) => {
  return { status: 200, body: { ok: true, who: ctx.query.who ?? "world" } };
});
```

Mount point:

```http
ANY /api/realms/<realm>/apps/<app>/custom/<path>
```

`ctx` has `{ method, path, query, headers, body, auth }`. Returning `undefined`/`null` ⇒ 204. Returning `{ status, body, headers }` is sent as-is. Throwing returns a 500 with the error message in `body.error`.

## Cron jobs

```ts
$app.cron("* * * * *", () => {
  $app.log("tick");
});

$app.cron("0 3 * * *", () => {
  // daily 03:00 — clean stale records
});
```

Five-field cron expression. Tasks are scheduled in the runtime; restarts re-schedule them from disk; a reload cancels old tasks before re-registering.

CPU and memory bounds for cron handlers are the same as for lifecycle hooks (see "Sandbox" below).

## `$app` records API

```ts
// Find one
const post = $app.records.findOne("posts", id);

// Find many by filter
const pinned = $app.records.findRecordsByFilter("posts", "pinned = true");

// Create
const rec = $app.records.create("posts", { title: "x" });

// Update
$app.records.update("posts", id, { pinned: false });

// Delete
$app.records.delete("posts", id);
```

All five run synchronously from the JS side (the Rust runtime parks the QuickJS thread and runs the async DB call on a dedicated executor).

## `$app.mailer.send`

Send mail through the configured SMTP relay:

```ts
$app.onRecordAfterCreate("orders", (rec) => {
  $app.mailer.send({
    to:      rec.fields.email,
    subject: "Order received",
    html:    `<p>Thanks for ordering <b>${rec.fields.item}</b>.</p>`
  });
});
```

The mailer is **rate-quoted per (realm, app)** — a runaway loop can't flood the relay. The cap is the `mailer.daily_cap` policy.

When no SMTP relay is configured, the server falls back to a `LogMailer` that traces the message but doesn't deliver it. Fine for dev; **don't** ship to prod without `[mail.smtp]` set.

Hooks: `$app.onMailerBeforeSend(fn)` lets you mutate the message; `$app.onMailerAfterSend(fn)` is a notification.

## Realtime publish

```ts
$app.realtime.publish("posts", { type: "custom", payload: { foo: 1 } });
```

Anything published lands in the realtime broker and is delivered to SSE/WebSocket subscribers on the matching collection.

## Sandbox

Each `(realm, app)` runs in its own QuickJS context with limits **bounded by [hierarchical policies](/concepts/hierarchical-policies)**:

| Policy | Meaning |
|---|---|
| `hooks.cpu_ms` | CPU deadline per hook invocation |
| `hooks.memory_mb` | Memory ceiling per VM |
| `hooks.network_allow` | Allowlist of hostnames `$app.http.fetch` can reach |
| `hooks.fs_allow` | Allowlist of paths for filesystem access (off by default) |

Hooks **cannot** import other files, spawn processes, or touch the database outside of `$app.records`. The runtime is isolated by design.

## TypeScript

`.ts` files are run through [`swc`](https://swc.rs) TypeScript-strip at load time. Types are erased, no type-checking is done at runtime — your editor's tsserver is your safety net.

If you want type completion against `$app`, add this to your editor's TS project:

```ts
// rustbase.d.ts
declare const $app: {
  log(msg: string): void;
  records: {
    findOne(coll: string, id: string): Record;
    findRecordsByFilter(coll: string, filter: string): Record[];
    create(coll: string, fields: Record["fields"]): Record;
    update(coll: string, id: string, fields: Record["fields"]): Record;
    delete(coll: string, id: string): void;
  };
  mailer: { send(msg: { to: string; subject: string; html?: string; text?: string }): void };
  realtime: { publish(coll: string, event: unknown): void };
  http: { fetch(url: string, init?: RequestInit): Response };
  cron(expr: string, fn: () => void): void;
  routerAdd(method: string, path: string, handler: (ctx: Ctx) => Resp): void;
  onRecordAfterCreate(coll: string, fn: (rec: Record) => void): void;
  // ...etc
};

type Record = { id: string; collection: string; fields: Record<string, unknown>; created_at: string; updated_at: string };
type Ctx = { method: string; path: string; query: Record<string, string>; headers: Record<string, string>; body: unknown; auth: { id: string; role: string } };
type Resp = { status?: number; body?: unknown; headers?: Record<string, string> } | undefined | null;
```

## Editing through the dashboard

The dashboard's **Hooks** tab edits the files in place. Saving triggers a reload and surfaces any compile errors next to the editor. The same surface is reachable via REST — see [hook source files](/reference/rest-api#hook-source-files).

## Errors

A hook that throws **does not** fail the underlying request. The exception is captured into `__rb_errors`, logged via `tracing::error!`, and surfaced in the dashboard's reload outcome and the audit log. That's intentional: hooks are post-write observers, not gates (except `Before*` hooks, which can veto by returning `false` or throwing).
