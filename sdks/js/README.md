# @rustbase/client

Official JavaScript / TypeScript client for [RustBase](https://github.com/pjonaszik/rustbase) — a multi-tenant Backend-as-a-Service in Rust.

```sh
# Once published:
bun add @rustbase/client
# or
npm install @rustbase/client
```

## Quick start

```ts
import { RustBase } from '@rustbase/client';

const rb = new RustBase({
    baseUrl:   'https://api.example.com',
    workspace: 'acme',
    onSessionChange: (s) => {
        if (s) localStorage.setItem('rb_session', JSON.stringify(s));
        else   localStorage.removeItem('rb_session');
    },
});

// Login (handles the MFA branch).
const result = await rb.auth.login({ email, password });
if (result.kind === 'mfa') {
    await rb.auth.completeMfa(result.mfaToken, totpCode);
}

// Records.
const notes = rb.app('mobile').collection('notes');
const list  = await notes.list({ filter: 'pinned = true', sort: '-updated_at' });
const note  = await notes.create({ title: 'hello' });
await notes.update(note.id, { pinned: true });
await notes.delete(note.id);

// Files.
const file = await rb.app('mobile').files.upload(blob);
await notes.update(note.id, { cover: file.id });
```

## Sessions

The client owns one in-memory `Session = { accessToken, refreshToken, user }`. Persistence is the caller's responsibility — supply `onSessionChange` to write the session into `localStorage`, a cookie, or your own storage. Restore on construction via the `session` option.

A 401 on any authenticated call triggers ONE refresh attempt against `POST /auth/refresh`; on success the original call is replayed transparently. On refresh failure the session is cleared and the original 401 is surfaced to the caller as a `RustBaseError`.

## Error handling

Every non-2xx response throws `RustBaseError` with:

```ts
{ status: number, code: string, message: string, body?: unknown }
```

`code` mirrors the server's `ErrorBody.code` when the body is JSON; transport-layer failures (DNS, ECONNRESET, etc.) get `code: 'network'` with `status: 0`.

## Development

```sh
# from sdks/js/
bun install
bun run test         # vitest, mock fetch
bun run build        # tsc → dist/
```

The SDK targets the canonical OpenAPI spec at [`../../docs/reference/openapi.yaml`](../../docs/reference/openapi.yaml) (also served by every running RustBase binary at `GET /openapi.yaml`).

## License

MIT OR Apache-2.0
