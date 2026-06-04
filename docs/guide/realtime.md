# Realtime

Every collection publishes lifecycle events on every successful write. Subscribers consume those events over **Server-Sent Events** (SSE).

## Subscribe

```http
GET /api/realms/:realm/apps/:app/collections/:coll/events
Accept: text/event-stream
Authorization: Bearer <token>
```

```sh
curl -N -H "authorization: Bearer $TOKEN" \
  http://localhost:8080/api/realms/acme/apps/web/collections/posts/events
```

The connection stays open and streams events as they happen.

## Event shape

```
event: record_created
data: {"id":"01HXY...","collection":"posts","fields":{...},"created_at":"...","updated_at":"..."}

event: record_updated
data: {"id":"01HXY...","collection":"posts","fields":{...},...}

event: record_deleted
data: {"id":"01HXY..."}
```

`event:` is the SSE event name; `data:` is the JSON payload. Heartbeats (`:keepalive`) are sent every 15 seconds so reverse proxies don't reap idle connections.

## Filtering

Subscriptions are per-collection. To narrow further, filter on the client side using the record id or fields — the broker doesn't run filter expressions on the publish side.

If you need per-record events specifically, watch the whole collection and filter:

```js
const es = new EventSource(`/api/realms/acme/apps/web/collections/posts/events`, {
  headers: { Authorization: `Bearer ${token}` }
});
es.addEventListener("record_updated", (e) => {
  const r = JSON.parse(e.data);
  if (r.id !== watching) return;
  applyPatch(r);
});
```

## Browser auth note

EventSource in the browser **cannot** set custom headers, including `Authorization`. Two options:

1. Use a polyfill (`event-source-polyfill`) that wraps `fetch` and re-implements SSE — supports headers.
2. Pass the token as a query string param: `?access_token=...` (the handler accepts this fallback). Be aware tokens then end up in access logs.

The dashboard uses option 1.

## Publish from JS hooks

```ts
$app.realtime.publish("posts", { type: "custom", payload: { foo: 1 } });
```

Any object you publish is delivered as an event named `custom` (or whatever string you set as `type`). Useful for app-level events that aren't a record write — chat messages, presence pings, notifications.

## Under the hood

`rustbase-realtime` is an in-process `tokio::sync::broadcast` channel keyed by `(realm, app, collection, optional_record_id)`. The API layer is a thin SSE wrapper.

Limitations:

- **Single-node.** Two RustBase instances don't share a broker. If you scale out (you really shouldn't, but…), put a sticky-session load balancer in front.
- **At-most-once.** A subscriber that disconnects between events doesn't get them on reconnect — there's no log/journal. If you need durable subscriptions, poll the collection.
- **No filters on the broker.** Apply filters client-side.

These are the standard SQLite-single-writer tradeoffs. For the 99% case (a single process serving an app), they're invisible.
