# Error codes

Every error response has the same shape:

```json
{ "code": "not_found", "message": "record not found: posts/01HXY..." }
```

The HTTP status code is sufficient for clients that don't care about the specifics; `code` lets you branch on the cause.

## Status → code mapping

| HTTP | `code` | When |
|---|---|---|
| 400 | `validation` | Request body failed validation (missing required field, wrong type, etc.). |
| 400 | `policy_violation` | A write tried to set a value outside the cascade-bound. |
| 400 | `unknown_column` | Filter referenced a column the collection doesn't have. |
| 400 | `parse_error` | Filter expression couldn't be parsed. |
| 401 | `unauthorized` | Missing or invalid `Authorization` header. |
| 401 | `bad_credentials` | Login failed: wrong password / unknown email / wrong OTP. |
| 401 | `mfa_required` | Returned with **202** during the two-step TOTP login. The body has `{challenge_id}`; follow up with `/auth/users/login/totp`. |
| 403 | `forbidden` | Authenticated but not allowed (scope / role / access rule). |
| 404 | `not_found` | The target object doesn't exist. `message` says what was missing. |
| 404 | `workspace_not_found` | Specifically the workspace in the URL doesn't exist. |
| 404 | `app_not_found` | Specifically the app in the URL doesn't exist. |
| 409 | `conflict` | Duplicate id, duplicate email, master admin already set, etc. |
| 413 | `payload_too_large` | File upload exceeds the storage cap. |
| 422 | (json) | Request couldn't be deserialized at all. Axum/Serde rejection. |
| 429 | `too_many_requests` | Per-IP rate limit hit, *or* per-subject auth lockout active. The response always carries a `Retry-After` header (seconds). See [Configuration → rate_limit / lockout](../guide/configuration.html#full-reference). |
| 500 | `internal` | Unhandled error path. Check server logs. |
| 503 | `uninitialized` | Server hasn't completed first-run setup. Only `/healthz` and `/_/setup` are reachable. |

## Validation errors

`validation` errors are produced by the [`validator`](https://docs.rs/validator) crate. The `message` field includes all field errors joined together, e.g.:

```
"email: invalid email; password: length(min=8): is too short"
```

If you need structured field-level errors, the dashboard parses this format; clients are encouraged to surface the whole message verbatim or split on `; ` for per-field lines.

## Policy-violation errors

```json
{
  "code":    "policy_violation",
  "message": "field=password.length value={min:4,max:64} outside bound master={min:8,max:128}"
}
```

These come from the cascade engine. The `message` always names the field, the offending value, and the parent scope's bound that was violated.

## What server logs say

For every non-2xx response, the server emits a structured `tracing::warn!` with the request id, the workspace, the app, and the inner error. Look there for the stack trace and the underlying `sqlx`/`object_store` error when `code: internal` shows up.

```
RUST_LOG=info,rustbase_api=debug ./rustbase
```

…will get you the per-handler error reason without flooding you with `sqlx` chatter.

## Idempotency

None of the destructive endpoints (`DELETE /api/...`) support an `Idempotency-Key` header today. Retries are safe (deleting a missing thing → 404, no destructive side-effect). For creates, generate the id client-side if you need idempotent retries — RustBase will respect the supplied `id` field on most create endpoints.
