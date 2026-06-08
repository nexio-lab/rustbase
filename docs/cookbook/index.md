# Cookbook

Practical recipes for builders. Each page solves one concrete problem with working code, then explains why the snippets are shaped the way they are.

The recipes assume you already have a workspace and an app — if not, walk through [Getting started](/guide/getting-started) and [First app, end-to-end](/guide/first-app) first.

## Auth

- [End-to-end sign-up flow](/cookbook/auth-flow) — register a user, verify the email via OTP, log in, refresh, and revoke. Includes the recommended cookie strategy for browser clients.
- [OAuth login (Google)](/cookbook/oauth-google) — configure a provider in the dashboard, build the redirect link from your client, exchange the callback for a RustBase session.

## Data

- [Filter and paginate records](/cookbook/filter-paginate) — power-user queries via the `?filter=` expression syntax, plus the pagination contract.
- [Upload and attach a file](/cookbook/files) — multipart upload, a `file` field on a collection, and serving the binary back through a signed URL.

## Realtime

- [Subscribe with a server-side filter](/cookbook/realtime) — open a WebSocket against `?filter=…` and have the server drop events that don't match. The same shape works for SSE.

## Hooks

- [Add a custom HTTP route](/cookbook/custom-route) — register a JS hook that mounts a route on the app's API surface, validates input, and reads + writes records via `$app.records`.
