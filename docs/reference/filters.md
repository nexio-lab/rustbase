# Filter syntax

RustBaas ships a unified filter language used by:

- `?filter=` query strings on list endpoints.
- Per-collection access rules (the `filter` template).
- `$app.records.findRecordsByFilter(...)` inside JS hooks.

The parser is `nom`-based, IO-free, and lives in `rustbase-core`. It produces a `FilterNode` AST that `rustbase-db` translates to a parameterized SQL `WHERE` clause — **no string interpolation of user input**, every literal becomes a bound parameter.

## Operators

| Operator | Meaning |
|---|---|
| `=`, `!=` | equality |
| `<`, `<=`, `>`, `>=` | comparison (numeric, datetime, text) |
| `~` | substring match (case-insensitive) — text only |
| `!~` | substring non-match |
| `&&` | logical AND |
| <code>&#124;&#124;</code> | logical OR |
| `!(...)` | logical NOT |
| `( ... )` | grouping |

Precedence: `!` > `=` / `~` / `<` … > `&&` > <code>&#124;&#124;</code>.

## Literals

```
"string in double quotes"
'string in single quotes'
42                  # integer
3.14                # float
true / false
null
"2026-05-27T10:00:00Z"   # datetime — RFC 3339 strings
```

## Examples

```
pinned = true
title ~ "Hello"
age >= 18 && age < 65
status = "active" && !(deleted)
tag = "intro" || tag = "tutorial"
created_at >= "2026-01-01T00:00:00Z"
owner = @request.auth.id           # in an access rule
```

## `@request` inside access rules

When the filter runs as an access rule, the request context is exposed:

| Variable | Value |
|---|---|
| `@request.auth.id` | the authenticated principal's id (empty string for anonymous) |
| `@request.auth.email` | the principal's email (empty for non-end-users) |
| `@request.auth.role` | one of `master_admin`, `realm_admin`, `app_admin`, `user`, `""` |
| `@request.body.<field>` | the field's value on the incoming write (for create/update only) |

So a "user can only update their own posts" rule is:

```
@request.auth.id != "" && author = @request.auth.id
```

## Validation

- Unknown column names → **400** (`unknown column: <name>`).
- Type mismatches (`age = "not a number"`) → **400**.
- Unclosed strings or stray operators → **400** with the parser's position.

The same parser runs in the dashboard for live validation as you type a filter — typos are caught before you hit Search.

## Performance notes

- Equality and range filters use the columns' default index when SQLite picks one. `text ~ "..."` uses `LIKE '%foo%'` and does **not** use an index — be mindful of large tables.
- Composed filters fold into a single `WHERE` clause; the engine doesn't issue N+1 round-trips.
- The clamp on `per_page` (max 200) keeps result-set sizes predictable; for bigger pulls iterate pages.
