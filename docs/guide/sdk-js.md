# JavaScript / TypeScript SDK

`@rustbase/client` is the official browser / Node / Bun client. It targets the [OpenAPI 3.1 spec](/reference/openapi) RustBase serves at `GET /openapi.yaml`, with a hand-written ergonomic surface on top.

## Install

```sh
bun add @rustbase/client
# or
npm install @rustbase/client
```

The package lives at [`sdks/js/`](https://github.com/pjonaszik/rustbase/tree/main/sdks/js) in the main repo until the first published release. To use the in-tree source for now:

```sh
bun add github:pjonaszik/rustbase#main --filter sdks/js
```

## Construct

```ts
import { RustBase } from '@rustbase/client';

const rb = new RustBase({
    baseUrl:   'https://api.example.com',
    workspace: 'acme',

    // Restore a previously persisted session, if any.
    session: JSON.parse(localStorage.getItem('rb_session') ?? 'null'),

    // Persist every change. Called on login, refresh, and logout.
    // Argument is `null` on logout.
    onSessionChange: (s) => {
        if (s) localStorage.setItem('rb_session', JSON.stringify(s));
        else   localStorage.removeItem('rb_session');
    },
});
```

The client owns one in-memory session. Persistence is yours: the SDK never touches `localStorage`, cookies, or anything outside its own object so it works the same in the browser, Node, and Bun.

## Auth

```ts
// Register, verify, login.
await rb.auth.register({ email, password });
await rb.auth.requestVerification({ email });
await rb.auth.confirmVerification({ email, code });

const result = await rb.auth.login({ email, password });
if (result.kind === 'mfa') {
    // The user has TOTP enabled — pair the mfa_token with the
    // current code and complete the login.
    await rb.auth.completeMfa(result.mfaToken, totpCode);
}

// After login, `rb.currentSession` is the active session.

await rb.auth.logout();
```

## Records

```ts
const notes = rb.app('mobile').collection('notes');

const list = await notes.list({
    filter:  'pinned = true',
    sort:    '-updated_at',
    page:    1,
    perPage: 30,
});

const note = await notes.create({ title: 'hello', pinned: true });
await notes.update(note.id, { pinned: false });
await notes.delete(note.id);
```

## Files

```ts
const blob = new Blob([buffer], { type: 'image/png' });
const file = await rb.app('mobile').files.upload(blob);
await notes.update(noteId, { cover: file.id });

// Serve URL for an <img src>.
const src = rb.app('mobile').files.serveUrl(file.id);
```

## Auto-refresh

A 401 on any authenticated call triggers ONE refresh attempt against `POST /auth/refresh`. On success the SDK swaps in the new tokens and replays the original request transparently — your code sees the eventual response. On refresh failure the session is cleared (`onSessionChange(null)` fires) and the original 401 is surfaced as a `RustBaseError`.

## Errors

Every non-2xx response throws `RustBaseError`:

```ts
import { RustBaseError } from '@rustbase/client';

try {
    await notes.create({ title: '' });
} catch (e) {
    if (e instanceof RustBaseError) {
        console.error(e.status, e.code, e.message);
        // e.body — the parsed JSON body when present, useful for
        // structured validator errors.
    }
}
```

Codes mirror the server's `ErrorBody.code` — see [Error codes](/reference/errors). Transport-layer failures (DNS, ECONNRESET, CORS) get `code: 'network'` with `status: 0`.

## Realtime

```ts
const notes = rb.app('mobile').collection('notes');
const sub   = notes.subscribe({ filter: 'pinned = true' });

sub.on('open',   ()         => console.log('connected'));
sub.on('close',  (info)     => console.log('closed', info.code, info.willReconnect));
sub.on('error',  (err)      => console.error(err));

sub.on('record_created', (record) => insert(record));
sub.on('record_updated', (record) => replace(record));
sub.on('record_deleted', (id)     => evict(id));

// Later:
sub.close();
```

What happens under the hood:

- The WebSocket URL rewrites the SDK's `baseUrl` from `http(s)` to `ws(s)` and appends `?token=<accessToken>&filter=…`. The browser `WebSocket` constructor can't set a `Authorization` header — the server accepts the token via query for this endpoint only.
- The wrapper **reconnects with jittered exponential backoff** on every close (500 ms → 1 s → 2 s → … capped at 30 s, plus up to 500 ms jitter). The wait-loop stops only when you call `sub.close()`.
- On policy-violation closes (codes `1008`, `4001`, `4003`), the wrapper **calls `auth.refresh()` first** so the next connect carries a fresh token. Refresh failure surfaces via `onSessionChange(null)` and the next reconnect attempt errors with `code: 'no_session'`.
- Events that arrive between disconnect and reconnect are LOST — the broker keeps no history. On reconnect, issue a fresh `GET …/records` to backfill.

## What's not (yet) in the SDK

- Admin operations (workspace / app / collection CRUD, schema PATCH, OAuth provider config, audit, hierarchical policies). The dashboard handles these today; the SDK will follow as the OpenAPI spec grows.
