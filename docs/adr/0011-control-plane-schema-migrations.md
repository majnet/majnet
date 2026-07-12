# ADR 0011 — Control-plane schema migrations (refinery, SQLite)

**Status:** accepted (implemented)
**Date:** 2026-07-12

## Context

The bot and reconciler each keep a small SQLite database (`bot.sqlite`,
`reconciler.sqlite`) for the state git doesn't hold: webhook-delivery dedup, an
audit log, the release store, in-flight import status, ephemeral-stack tracking,
data-migration idempotency. The schema was managed ad-hoc in `Store::open` —
`CREATE TABLE IF NOT EXISTS` for tables plus one-off `ALTER TABLE ADD COLUMN`
"poor-man's migrations" for later columns.

That approach bit us in production: the `imports` table shipped without a
`request` column, then gained one — but `CREATE TABLE IF NOT EXISTS` can't add a
column to an existing table, so the live DB was missing it and `begin_import`
failed at runtime (`no column named request`). Every column addition was a
manual, easy-to-forget `ALTER` guarded by "ignore the error." We need real,
versioned, tracked migrations.

## Decision

**Keep SQLite; adopt [`refinery`](https://crates.io/crates/refinery) for
versioned migrations.** SQLite fits the control plane's design (§ "carries no
state git doesn't") — low-volume, single-writer, embedded, zero-ops — better
than a stateful Postgres dependency, which would also create a bootstrap-order
problem (the reconciler *provisions* Postgres engines) and more credential
surface. Postgres was considered and rejected for now; refinery works over both
rusqlite and postgres, so a later switch is low-cost.

- Migrations are ordered SQL files per crate: `crates/<crate>/migrations/
  V{N}__{name}.sql`. `refinery::embed_migrations!("migrations")` bakes them into
  the binary at **compile time** (no runtime file dependency; `COPY crates` in
  the Dockerfile brings them into the CI build).
- `Store::open` runs `embedded::migrations::runner().run(&mut conn)` on startup.
  refinery tracks applied migrations in its `refinery_schema_history` table and
  runs only the new ones.

### Baselining the pre-refinery live databases

`V1__baseline.sql` uses `CREATE TABLE IF NOT EXISTS` for the **current full
schema** (including columns previously added by `ALTER` hacks). This makes V1 a
safe baseline in both directions:

- **Existing live DB** (tables already present, no refinery history): V1 no-ops,
  refinery records it as applied. The DB is now refinery-managed.
- **Fresh DB**: V1 creates the full schema.

Future schema changes are **append-only** new files (`V2__…`, plain DDL — no
`IF NOT EXISTS` needed, since the ordering + history guarantee they run once).

## Consequences

- No more ad-hoc `ALTER` hacks; a schema change is a new `V{N}__*.sql` file.
- Compile-time embedding means the binary is self-contained; migrations run
  deterministically on every start.
- Minor divergence on pre-refinery DBs: legacy unused columns that were never
  dropped (e.g. `releases.migration_image`/`migration_command`, removed from the
  code in ADR 0009 rev 2) remain on old DBs. They're nullable + unreferenced;
  harmless. A fresh DB won't have them. A future `V2` could drop them if desired.

## How to add a migration

Drop `crates/<crate>/migrations/V{N}__short_name.sql` with the next number and
plain DDL. It runs once, in order, on the next start; refinery records it.
