# Filter and paginate records

A field guide to RustBase's query string for `GET /records` — what the filter language understands, how pagination is shaped, and the gotchas that fall out of running the same AST in three places (SQL, JS hooks, dashboard).

## What you'll build

A page that pulls a filtered, paginated slice of `notes` matching `pinned = true && updated_at > now-7d`, sorted newest-first.

## The endpoint

```http
GET /api/workspaces/:workspace/apps/:app/collections/:coll/records
  ?filter=<expression>
  &sort=<expression>
  &page=<n>
  &per_page=<m>
```

All four query parameters are optional. Without them you get **page 1, 30 rows, unsorted, unfiltered**.

## A real query

```sh
curl -s -G http://localhost:8080/api/workspaces/acme/apps/mobile/collections/notes/records \
  -H "authorization: Bearer $AT" \
  --data-urlencode 'filter=pinned = true && updated_at > "2026-05-30T00:00:00Z"' \
  --data-urlencode 'sort=-updated_at' \
  --data-urlencode 'page=1' \
  --data-urlencode 'per_page=50'
```

```json
{
  "items":       [ { "id": "…", "fields": { "title": "…", "pinned": true } }, … ],
  "page":        1,
  "per_page":    50,
  "total_items": 137,
  "total_pages": 3
}
```

## Filter grammar

The parser is the `nom`-based grammar in `rustbase-core::filter`. It produces a `FilterNode` AST that the SQL translator turns into a parameterised `WHERE` clause — there is **no string interpolation** of user input.

```text
expr        := term  ( ( "&&" | "||" ) term )*
term        := "!" term  |  "(" expr ")"  |  comparison
comparison  := ident op value
op          := "=" | "!=" | ">" | ">=" | "<" | "<=" | "like" | "in"
ident       := [a-z_][a-z0-9_]*
value       := "<string>" | <number> | true | false | null | <date>
                | "(" value ( "," value )* ")"           // for `in`
```

Examples that all parse:

```text
title like "intro%"
status in ("draft", "published")
!(archived = true)
created_at > "2026-01-01T00:00:00Z" && views >= 100
```

Strings are double-quoted. Numbers are unquoted. Booleans are `true`/`false`. RFC 3339 dates are quoted strings that the SQL layer compares correctly because the column was stored as ISO-8601 text on a SQLite `DATETIME`.

## Sort grammar

`sort=<field>` or `sort=-<field>` for descending. Multiple sorts comma-separated: `sort=-pinned,-updated_at`.

Only collection fields are sortable. There is no `RANDOM()` or `LENGTH(field)` — those would defeat the prepared-statement model.

## Pagination

- `page` is 1-based.
- `per_page` is capped at **200** (configurable via the hierarchical policy in a future release).
- `total_items` and `total_pages` are evaluated against the **filtered** set, not the whole collection. A page-1 request and a page-2 request can disagree on `total_items` if a concurrent write changed the set in between — clients should not rely on the count being stable mid-walk.

## Gotchas

- **Auth rules can shrink the filter.** Each collection has an access rule (also a `FilterNode`) that the server `AND`s onto the user's query — so a row that exists in the table but fails the rule never appears, never counts, never paginates. If you see fewer rows than you expect, check the access rule before you blame the filter.
- **`like` is case-sensitive** by default (matches SQLite's default). Use `lower(field) like "intro%"` instead — wait, no: there is no `lower()` function. Store the lowercase form in a separate field if you need case-insensitive search.
- **`in` is `IN (?, ?, …)`** under the hood, not a sub-query. The list must be literal values — `field in (other_field)` does NOT do what you'd want.

## Variations

- **Realtime + filter** — the same expression goes into the WebSocket `?filter=`; the broker drops events that don't match. See [the realtime recipe](/cookbook/realtime).
- **JS hook + filter** — `$app.records.findRecordsByFilter("notes", "pinned = true")` evaluates the same AST in-process via `FilterNode::matches(fields)`.
- **Dashboard + filter** — the filter bar on the records page parses client-side with the same grammar before hitting the server, so syntax errors get caught before round-tripping.
