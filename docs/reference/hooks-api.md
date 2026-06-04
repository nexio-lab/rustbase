# `$app` API reference

Inside a JS/TS hook file, `$app` is a global the runtime injects. This page is the exhaustive surface.

## Logging

```ts
$app.log(...args: unknown[]): void;
```

Each argument is stringified and joined with spaces; the resulting line lands in RustBase's `tracing::info!` output and in `drain_logs`.

## Records

```ts
$app.records.findOne(coll: string, id: string): Record | null;
$app.records.findRecordsByFilter(coll: string, filter: string): Record[];
$app.records.create(coll: string, fields: object): Record;
$app.records.update(coll: string, id: string, fields: object): Record;
$app.records.delete(coll: string, id: string): void;
```

`Record` is `{ id, collection, fields, created_at, updated_at }`. `fields` is a plain object whose keys are field names and whose values are the typed JSON the schema describes.

`findRecordsByFilter` accepts the same [filter syntax](/reference/filters) the REST API does.

## Mailer

```ts
$app.mailer.send(msg: {
  to:      string;
  cc?:     string | string[];
  bcc?:    string | string[];
  subject: string;
  text?:   string;
  html?:   string;
  from?:   string;     // overrides the configured From
}): void;
```

Synchronous from the JS side — the runtime awaits the send. Bounded by the `mailer.daily_cap` [policy](/guide/policies).

Hooks:

```ts
$app.onMailerBeforeSend((msg) => {
  msg.subject = "[Acme] " + msg.subject;
});
$app.onMailerAfterSend((msg, result) => {
  // result is { messageId } on success or { error } on failure
});
```

## Realtime

```ts
$app.realtime.publish(coll: string, event: unknown): void;
```

Any value is JSON-serialized and pushed to every SSE / WebSocket subscriber on the collection's channel.

## HTTP fetch

```ts
$app.http.fetch(url: string, init?: RequestInit): Response;
```

Issues an outbound HTTP request. Bounded by `hooks.network_allow` (an enum-set of hostnames; empty = no outbound). Returns a `Response`-shaped object: `{ status, headers, body, ok }` where `body` is the response body as a string.

## Cron

```ts
$app.cron(expr: string, fn: () => void): void;
```

Five-field cron expression in **server-local time**. Returns nothing — there's no `cancel`; reloading hooks rebuilds the schedule from scratch.

## Custom routes

```ts
$app.routerAdd(
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
  path: string,
  handler: (ctx: Ctx) => Resp
): void;
```

Mounted at `/api/workspaces/<workspace>/apps/<app>/custom<path>`. `path` starts with `/`.

```ts
type Ctx = {
  method:  string;
  path:    string;
  query:   Record<string, string>;
  headers: Record<string, string>;
  body:    unknown;       // JSON-parsed when content-type is application/json
  auth:    { id: string; role: string };
};

type Resp =
  | { status?: number; body?: unknown; headers?: Record<string, string> }
  | undefined
  | null;
```

Returning `undefined` or `null` ⇒ 204. Throwing returns 500.

## Record lifecycle hooks

```ts
$app.onRecordBeforeCreate(coll: string, fn: (rec: Record) => void | false);
$app.onRecordAfterCreate (coll: string, fn: (rec: Record) => void);
$app.onRecordBeforeUpdate(coll: string, fn: (existing: Record, patch: object) => void | false);
$app.onRecordAfterUpdate (coll: string, fn: (rec: Record) => void);
$app.onRecordBeforeDelete(coll: string, fn: (rec: Record) => void | false);
$app.onRecordAfterDelete (coll: string, fn: (rec: Record) => void);
```

Returning `false` from a `Before*` hook vetoes the operation; the REST request gets a 400.

## User lifecycle hooks (app-scoped)

```ts
$app.onUserBeforeLogin   (fn: (user) => void | false);
$app.onUserAfterLogin    (fn: (user) => void);
$app.onUserAfterRegister (fn: (user) => void);
```

These fire only on the target app's runtime — the one whose `/apps/:app/auth/users/...` endpoint handled the request. Sibling apps in the same workspace don't see the event; end-users are per-app, so cross-app fan-out wouldn't make sense.

## Misc

```ts
$app.request: { id: string; method: string; path: string; headers: Record<string,string>; auth: {id,role} } | null;
```

The current request context, when a hook is running inside a request handler. `null` for cron jobs and other non-request triggers.

```ts
$app.env: Record<string, string>;
```

A read-only snapshot of the server's environment variables. Useful for grabbing things like an API key without hard-coding it in the hook file. Filtered to keys you allow via `hooks.env_allow` (enum_set policy).
