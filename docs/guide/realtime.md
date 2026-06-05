# Realtime

Every collection publishes lifecycle events on every successful write. Subscribers consume those events over **Server-Sent Events** (SSE) or **WebSocket** — same broker, same payloads, same auth.

## Subscribe (SSE)

```http
GET /api/workspaces/:workspace/apps/:app/collections/:coll/events
Accept: text/event-stream
Authorization: Bearer <token>
```

```sh
curl -N -H "authorization: Bearer $TOKEN" \
  http://localhost:8080/api/workspaces/acme/apps/web/collections/posts/events
```

The connection stays open and streams events as they happen.

## Subscribe (WebSocket)

```http
GET /api/workspaces/:workspace/apps/:app/collections/:coll/events/ws
Upgrade: websocket
Authorization: Bearer <token>
```

```js
const ws = new WebSocket(
  `ws://localhost:8080/api/workspaces/acme/apps/web/collections/posts/events/ws`,
  [],
);
ws.addEventListener('message', (m) => {
  const ev = JSON.parse(m.data);
  // ev.kind is "record_created" | "record_updated" | "record_deleted"
  console.log(ev);
});
```

Same auth + filter semantics as the SSE route. The server is push-only after the upgrade; messages from the client are drained but never interpreted (sending `close` ends the subscription cleanly).

## Event shape

SSE encodes the event name + JSON payload:

```
event: record_created
data: {"kind":"record_created","record":{"id":"01HXY...","collection":"posts","fields":{...},"created_at":"...","updated_at":"..."}}

event: record_updated
data: {"kind":"record_updated","record":{"id":"01HXY...","collection":"posts","fields":{...},...}}

event: record_deleted
data: {"kind":"record_deleted","id":"01HXY..."}
```

WebSocket sends one JSON frame per event with the same `data:` body, so the same `JSON.parse` handles both transports.

Heartbeats (`:keepalive`) are sent every 15 seconds on the SSE stream so reverse proxies don't reap idle connections. WebSocket relies on the underlying ping/pong frames.

## Filtering

Both endpoints accept an optional `?filter=<expression>` query parameter using the same syntax as the records list:

```sh
# Stream only events for records whose `status` is "open"
curl -N -H "authorization: Bearer $TOKEN" \
  "http://localhost:8080/api/workspaces/acme/apps/web/collections/posts/events?filter=status%20%3D%20%27open%27"
```

The filter is parsed and **evaluated server-side** on every event's record. Three semantics worth knowing:

- `record_deleted` events have no record body and always pass the filter — subscribers can evict cached rows even when the underlying row stopped matching before delete.
- For collections with an access rule that's a template (e.g. `owner = @request.auth.id`), the rule is materialised against the subscribing principal and **intersected** with any client-supplied filter — the broker only forwards events that satisfy both.
- A row whose values change *out of* a filter's match set still emits a `record_updated` frame the client sees. The client should treat `updated` as "may no longer match; re-check or drop".

## Publish from JS hooks

```ts
$app.realtime.publish("posts", { type: "custom", payload: { foo: 1 } });
```

Any object you publish is delivered as an event named `custom` (or whatever string you set as `type`). Useful for app-level events that aren't a record write — chat messages, presence pings, notifications.

## Browser auth note

`EventSource` in the browser **cannot** set custom headers, including `Authorization`. Two options:

1. Use a polyfill (`event-source-polyfill`) that wraps `fetch` and re-implements SSE — supports headers.
2. Pass the token as a query-string param: `?access_token=...` (the handler accepts this fallback). Be aware tokens then end up in access logs.

The dashboard uses option 1.

The WebSocket route accepts `Authorization` natively, so browser clients that already manage a WS connection can skip the polyfill entirely.

## Under the hood

`rustbase-realtime` is an in-process `tokio::sync::broadcast` channel keyed by `(workspace, app, collection)`. The API layer is a thin SSE / WebSocket wrapper that subscribes to the same channel and filters events before forwarding to clients.

Limitations:

- **Single-node.** Two RustBase instances don't share a broker. If you scale out (you really shouldn't, but…), put a sticky-session load balancer in front.
- **At-most-once.** A subscriber that disconnects between events doesn't get them on reconnect — there's no log/journal. If you need durable subscriptions, poll the collection.
- **Filters are best-effort.** Heavy client filters still cost broker fan-out: every event is materialised + matched per connection. Don't open thousands of high-cardinality filter subscriptions on a single collection.

These are the standard SQLite-single-writer tradeoffs. For the 99% case (a single process serving an app), they're invisible.
