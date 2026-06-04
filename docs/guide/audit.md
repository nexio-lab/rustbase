# Audit log

Every significant change writes a row to the matching scope's `audit_log` table. The log is append-only — no UPDATE, no DELETE — so the history is always intact.

## What's recorded

| Action | Where |
|---|---|
| `policy_set` | scope where the policy was set |
| `policy_clamped` | each child whose value was clamped on a parent tighten |
| `policy_deleted` | scope where the policy was deleted |
| `oauth_provider_set` | app |
| `oauth_provider_deleted` | app |
| `workspace_admin_added` | system |
| `app_admin_added` | workspace |
| `user_force_verified` | app |
| `user_totp_reset` | app |
| `user_deleted` | app |
| `workspace_deleted` | system |
| `app_deleted` | workspace |
| `collection_deleted` | app |
| `access_rule_set` | app |
| `hooks_reloaded` | app |

The list grows; the dashboard's **Audit** tab reflects whatever's in the table.

## Entry shape

```json
{
  "id":      42,
  "ts":      "2026-05-27T10:00:00Z",
  "actor":   "01HXY-admin-id-or-system",
  "action":  "policy_clamped",
  "target":  "password.length",
  "details": { "before": {"kind":"range","min":6,"max":12}, "after": {"kind":"range","min":8,"max":12} }
}
```

- `actor` is `null` for cascade-initiated entries (the system wrote them, not a person).
- `target` is whatever field/id the action was about.
- `details` is the action-specific JSON payload — schema isn't enforced, but every action publishes a stable shape.

## Read the log

```http
GET /api/system/audit?page=&per_page=&action=&actor=
GET /api/workspaces/:workspace/audit?...
GET /api/workspaces/:workspace/apps/:app/audit?...
```

- `action` is a case-insensitive substring match.
- `actor` is an exact match.
- Pagination is the standard `?page=&per_page=` (max 200).

Newest entries first.

## Retention

The `audit.retention_days` [policy](/guide/policies) controls how long entries survive before pruning. There's no scheduled pruning job in-binary today — set up a hook with `$app.cron` if you want one:

```ts
$app.cron("0 4 * * *", () => {
  $app.db.exec(
    "DELETE FROM audit_log WHERE ts < datetime('now', '-90 days')"
  );
});
```

(Not yet — `$app.db.exec` is on the roadmap. For now, prune at the SQLite level: stop the server and `sqlite3 data/system.db "DELETE FROM audit_log WHERE ts < datetime('now','-90 days')"`.)

## What it isn't

- **Not** a per-record audit. Records are versioned only as far as `created_at` / `updated_at` go. If you want a full change history, write hooks that copy old/new pairs into a `_history` collection.
- **Not** an access log. Authenticated request → response pairs come out of `tower_http::trace::TraceLayer` and are in `RUST_LOG` instead.
- **Not** mutable. The dashboard never offers a "delete this entry" button — that's by design.
