# Policies (in practice)

This page is the operator's view of the policy system. For the concept and the cascade rules, read [hierarchical policies](/concepts/hierarchical-policies) first.

## Three scopes, same UI

The dashboard's **Policies** tab exists in three places:

| Path | Scope | Who can edit |
|---|---|---|
| `/system/policies` | System (master scope) | Master admins |
| `/workspaces/<workspace>/policies` | Workspace | Master + workspace admins |
| `/workspaces/<workspace>/apps/<app>/policies` | App | Master + workspace + app admins |

All three render the same editor. The differences are which bound you're allowed to **set** and which bound you're allowed to **violate** (you can't).

## The editor

For each policy field, you see:

- The current value (or "unset" — inherits the parent).
- The kind (`range` / `toggle` / `enum_set` / `free`).
- A widget appropriate to the kind (slider for range, switch for toggle, multi-select for enum_set, free-text for free).
- A "Save" button. Disabled unless the value fits inside the parent's bound.

A successful save triggers the cascade engine. If your change tightens a bound and a child violates it, the engine clamps the child and the dashboard surfaces an **amber banner** listing every clamped entry.

## REST surface

```http
GET    /api/system/policies
GET    /api/system/policies/:field
PUT    /api/system/policies/:field        # body: PolicySpec
DELETE /api/system/policies/:field

GET    /api/workspaces/:workspace/policies
GET    /api/workspaces/:workspace/policies/:field
PUT    /api/workspaces/:workspace/policies/:field
DELETE /api/workspaces/:workspace/policies/:field

GET    /api/workspaces/:workspace/apps/:app/policies
GET    /api/workspaces/:workspace/apps/:app/policies/:field
PUT    /api/workspaces/:workspace/apps/:app/policies/:field
DELETE /api/workspaces/:workspace/apps/:app/policies/:field
```

`PUT` body is a `PolicySpec`:

```json
{ "kind": "range",    "min": 6,  "max": 64 }
{ "kind": "toggle",   "state": "locked", "value": true }
{ "kind": "toggle",   "state": "open",   "default": false }
{ "kind": "enum_set", "allowed": ["google", "github"] }
{ "kind": "free" }
```

`PUT` response includes a `cascaded` array describing any children that were auto-clamped — same data the dashboard's amber banner reads.

`DELETE` on a child returns it to "inherits from parent." `DELETE` on the master scope drops the field entirely (use with care — every child that depended on it now inherits "free / no bound").

## The fields that ship today

The exact list is discovered at runtime. The dashboard lists every field that has a row in the matching scope's `policies` table. Common ones:

```
password.length              range<u32>
password.require_special     toggle
tokens.access_ttl_sec        range<u32>
tokens.refresh_ttl_sec       range<u32>
rate.requests_per_min        range<u32>
oauth.providers              enum_set<string>
mailer.daily_cap             range<u32>
storage.max_upload_mb        range<u32>
hooks.cpu_ms                 range<u32>
hooks.memory_mb              range<u32>
hooks.network_allow          enum_set<string>
hooks.fs_allow               enum_set<string>
audit.retention_days         range<u32>
```

You can introduce new fields **at any time** by `PUT`ting a new `:field`. The system doesn't care about the name — bounds and validation are kind-driven.

## A worked example

You want every workspace's password length to be **at least 8** characters.

1. As master, set the master bound:
   ```http
   PUT /api/system/policies/password.length
   { "kind": "range", "min": 8, "max": 128 }
   ```
2. Workspace `acme` previously had `{min: 6, max: 12}`. The cascade engine sees the conflict and clamps to `{min: 8, max: 12}`. An audit row lands in both the master and the `acme` workspace's `audit_log`.
3. The next time the `acme` workspace admin opens the dashboard, the **Audit** tab shows a `policy_clamped` entry telling them what changed.

This is what hierarchical policies are *for*: a master admin can tighten a control globally and trust that every child workspace and app catches up automatically, with a paper trail.
