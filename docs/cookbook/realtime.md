# Subscribe with a server-side filter

How to open a realtime stream that only emits the rows you care about. The same `FilterNode` AST that backs `?filter=` on the REST endpoint runs against every published event — events that don't match never cross the wire.

## What you'll build

A browser client that watches `notes` rows where `pinned = true` change in real time. The server drops every other event before it leaves the broker.

## 1. Open the WebSocket

The realtime endpoint mirrors the records list URL — `/events` instead of `/records`:

```ts
const ws = new WebSocket(
  'ws://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/events'
    + '?filter=' + encodeURIComponent('pinned = true')
    + '&token=' + encodeURIComponent(accessToken)
);
```

A few things in flight:

- **Filter** — same grammar as the REST `?filter=`. `nom`-parsed once on connect, evaluated against `record.fields` for every publish.
- **Token** — JWT carried as a query param because the `WebSocket` constructor in browsers can't set Authorization headers. The server validates it as if it were the `Authorization: Bearer` form.
- **Access rule** — the collection's access rule is `AND`-ed onto the user-supplied filter, exactly as on the REST side. A user without read permission gets `1006 Policy Violation` on the close frame.

## 2. Handle events

```ts
ws.addEventListener('message', (ev) => {
  const event = JSON.parse(ev.data);
  // { kind: "record_created" | "record_updated" | "record_deleted",
  //   record: { id, fields, updated_at } }
  switch (event.kind) {
    case 'record_created':
    case 'record_updated':
      replace(event.record);
      break;
    case 'record_deleted':
      evict(event.record.id);
      break;
  }
});
```

`record_deleted` events **always pass the filter**, even if the deleted row no longer matches `pinned = true`. The reason: a row that used to match but doesn't anymore (`pinned` flipped to `false`) still needs to disappear from the client's cache. The server can't tell the difference between "row left the filter" and "row was deleted" once the row is gone, so it lets every delete through and lets the client evict.

## 3. Reconnect on close

WebSockets close for any reason — network blip, server restart, an idle timeout in a proxy. The dashboard's pattern:

```ts
function connect() {
  const ws = new WebSocket(url);
  ws.addEventListener('close', () => {
    setTimeout(connect, 1000 + Math.random() * 2000);  // jittered backoff
  });
  ws.addEventListener('error', () => ws.close());
  return ws;
}
```

The broker doesn't retain history. A reconnecting client should issue a fresh `GET /records?filter=…` to backfill the gap before resuming the stream.

## SSE alternative

When you don't need to push back (most read-mostly UIs), Server-Sent Events is one less moving part:

```sh
curl -N -H "authorization: Bearer $AT" \
  "http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/events\
?filter=pinned%20%3D%20true"
```

Same filter syntax, same access-rule behaviour. SSE responses set `text/event-stream` and `data:` framed events. Browsers' `EventSource` reconnects automatically with exponential backoff and forwards the `Last-Event-ID` header on retry — useful for at-least-once delivery if you wire it up.

## Variations

- **Single-record subscription** — `?record=01HXY…` instead of a filter. The broker keys by `(workspace, app, collection, record_id)` and skips the AST evaluation entirely. Right shape when you want to watch one form's row.
- **Template-driven filter** — access rules can reference `@request.auth.id`, so `owner = @request.auth.id` automatically narrows the stream to the current user without the client knowing their own ID.
- **Hooks publish too** — `$app.realtime.publish("notes", record)` from a JS hook flows through the same filter check. Don't double-publish on top of a record write — the record handler already publishes.

## Gotchas

- **The broker is in-process.** Multiple `rustbase` instances behind a load balancer each have their own broker, and events don't cross the boundary. RustBase is a **single-binary, single-node** server by design — for multi-node, see the realtime guide.
- **Slow consumers get dropped.** Each subscription is a bounded `tokio::sync::broadcast` channel. A client that doesn't drain its inbox fast enough sees `1011 Internal Error` and reconnects. Don't subscribe and then `console.log` synchronously — buffer the events.
