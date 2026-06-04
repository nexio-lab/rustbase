# Hierarchical policies

Every config field that varies across workspaces or apps is a **policy**. A policy declares *what kind of bound* it has — a numeric range, a toggle, an enum set, or a free value — and the system walks master → workspace → app at write time to make sure every stored value fits.

## The four kinds

| Kind | Parent sets | Child can do |
|---|---|---|
| `Range<T>` | `[min, max]` + default | Pick any value inside, or **tighten** the range further. |
| `Toggle` | `Locked(true)` / `Locked(false)` / `Open(default)` | If `Open`, choose freely; if `Locked`, cannot change. |
| `EnumSet<T>` | Allowed set | Pick any **subset**; cannot add values. |
| `Free<T>` | Default | Override freely. |

A child can never escape its parent. App ⊆ workspace ⊆ master.

## Wire format

```json
// Range
{ "kind": "range", "min": 6, "max": 64 }

// Toggle
{ "kind": "toggle", "state": "locked", "value": true }
{ "kind": "toggle", "state": "open",   "default": false }

// EnumSet
{ "kind": "enum_set", "allowed": ["google", "github"] }

// Free
{ "kind": "free" }
```

## Auto-clamp on tighten

What happens when master narrows a bound *after* children have already set values?

The system **auto-clamps and audits**:

1. Locates every workspace and app whose stored value falls outside the new bound.
2. Clamps each non-compliant value to the new bound (or to the new default for removed enum values / `Locked` toggles), in **one transaction** across `system.db`, every affected `workspace.db`, and every affected app `data.db`.
3. Writes an entry to the master audit log AND each affected workspace/app audit log, so admins downstream see what happened the next time they open their dashboard.

Same machinery applies to workspace → app when a workspace tightens its bounds.

## Common policy fields

The shape of every policy is described above; the **specific fields** that ship today include:

| Field | Kind | Typical use |
|---|---|---|
| `password.length` | Range | Min/max characters for end-user passwords. |
| `password.require_special` | Toggle | Force at least one special character. |
| `tokens.access_ttl_sec` | Range | Access-token lifetime in seconds. |
| `tokens.refresh_ttl_sec` | Range | Refresh-token lifetime. |
| `rate.requests_per_min` | Range | Per-IP rate limit. |
| `oauth.providers` | EnumSet | Which OAuth providers are usable in this scope. |
| `mailer.daily_cap` | Range | Per-(workspace, app) cap on `$app.mailer.send` invocations. |
| `hooks.cpu_ms` | Range | CPU deadline per hook invocation. |
| `hooks.memory_mb` | Range | Memory ceiling per VM. |
| `audit.retention_days` | Range | How long audit entries survive before pruning. |

The dashboard's **Policies** tab (System / Workspace / App) is the easiest way to discover what fields exist and what their current bounds are. The same data is reachable through the REST API at `GET /api/{scope}/policies`.

## Who can edit what

- **Master config + bounds**: master admins only.
- **Workspace config + bounds for apps**: workspace admins (within master bounds) and master admins.
- **App config**: app admins (within workspace bounds), workspace admins, master admins.

A workspace admin who tries to set a value outside the master bound gets a `policy_violation` 400 — the chain is walked **at write time** so reads never have to recompute.

## Validation walk

Pseudo-code for what happens when a child writes:

```rust
fn write_app_policy(app, workspace, field, value) -> Result<()> {
    let master = system.get(field);
    let workspace  = workspace.get(field).unwrap_or(master);
    if !master.fits(value) { return Err(PolicyViolation { against: "master" }) }
    if !workspace.fits(value)  { return Err(PolicyViolation { against: "workspace" })  }
    app.set(field, value)?;
    audit.append("policy_set", details);
    Ok(())
}
```

…and on a *parent* tighten:

```rust
fn tighten_master_bound(field, new_spec) -> Result<()> {
    let txn = begin_transaction_across_all_scopes();
    system.set(field, new_spec)?;
    for workspace in all_realms {
        if !new_spec.fits(workspace.get(field)) {
            let clamped = new_spec.clamp(workspace.get(field));
            workspace.set(field, clamped)?;
            audit.append("policy_clamped", master_log);
            audit.append("policy_clamped", workspace_log);
        }
        for app in apps_in(workspace) {
            // same shape, against the workspace's (possibly already clamped) bound
        }
    }
    txn.commit()
}
```

That's the whole engine.

## When you'd reach for this

- Letting workspace admins **self-serve** without giving them a way to break the platform.
- Charging tiered customers different limits (`Range<u32>` for `rate.requests_per_min`).
- Locking off a regulated control globally (`Locked(true)` toggle) without touching every app.
- Letting an app admin opt in to a more permissive limit only if their workspace allows it.
