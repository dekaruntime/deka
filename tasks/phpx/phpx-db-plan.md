# PHPX DB System Plan

## Goals
- Keep PHPX user experience clean and typed.
- Keep host/runtime primitives loosely coupled from any single database.
- Ship `postgres` first for `linkhash`.
- Prove the architecture can support `sqlite` and `mysql` next.

## Architecture
- Host layer: generic DB capability endpoint.
- PHPX layer:
  - `db` package provides generic primitives.
  - canonical driver packages live under `db/*` (`db/postgres`, `db/mysql`, `db/sqlite`).
- Bridge:
  - PHPX public packages call internal modules only.
  - internal modules call the runtime host bridge.
  - Host handles action dispatch and returns JSON-compatible payloads.

## Host Contract (v1)
- `open`: `{ driver, config } -> { ok, handle, reused }`
- `query`: `{ handle, sql, params } -> { ok, rows }`
- `exec`: `{ handle, sql, params } -> { ok, affected_rows }`
- `begin`: `{ handle } -> { ok }`
- `commit`: `{ handle } -> { ok }`
- `rollback`: `{ handle } -> { ok }`
- `close`: `{ handle } -> { ok }`

Errors use:
- `{ ok: false, error: string }`

## Implementation Status (Current)
- Implemented: `op_php_db_call` host action bridge in `crates/modules_php/src/modules/php/mod.rs`.
- Implemented: host driver routes:
  - `postgres` (sync `postgres` crate)
  - `sqlite` (sync `rusqlite` crate)
  - `mysql` (sync `mysql` crate)
- Implemented: PHPX packages:
  - `php_modules/db` (generic primitives)
  - `php_modules/db/postgres` (driver wrapper)
  - `php_modules/db/sqlite` (driver wrapper)
  - `php_modules/db/mysql` (driver wrapper)
  - legacy top-level `php_modules/postgres|sqlite|mysql` compatibility proxies removed
- Implemented: canonical unprefixed exports in module namespaces:
  - `db`: `open`, `open_handle`, `query`, `query_one`, `rows`, `exec`, `affected_rows`, `begin`, `commit`, `rollback`, `close`, `stats`
  - `db/postgres`: `connect`, `query`, `query_one`, `exec`, `begin`, `commit`, `rollback`, `close`
  - `db/sqlite`: `connect`, `query`, `query_one`, `exec`, `begin`, `commit`, `rollback`, `close`
  - `db/mysql`: `connect`, `query`, `query_one`, `exec`, `begin`, `commit`, `rollback`, `close`
- Implemented: db facade row normalization now preserves native scalar types (`int|float|bool|string|null`) instead of forcing string conversion.
- Implemented: host-level per-handle statement cache tracking for `query/exec`, surfaced via `db.stats()` as `statement_cache_entries|hits|misses` (JSON + protobuf paths).
- Verified with `deka run` smoke scripts:
  - sqlite end-to-end `connect/exec/query/close` succeeds.
  - mysql module loads and returns structured errors when query fails.
  - postgres powers live `linkhash` API routes.

## PHPX API (v1)
- `db`:
  - `open/open_handle(driver, config): Result<DbOpenOk|int, string>`
  - `query/query_one/rows(handle, sql, params): Result<DbQueryOk|Object|array<Object>, string>`
  - `exec/affected_rows(handle, sql, params): Result<DbExecOk|int, string>`
  - `begin/commit/rollback/close(handle): Result<DbUnitOk, string>`
  - `stats(): Result<DbStatsOk, string>` (active handles + timing counters)
- `db/postgres|db/sqlite|db/mysql`:
  - `connect/query/query_one/exec/begin/commit/rollback/close`

## ORM Mapping Notes (Current)
- `array<T>` model fields map to `JSONB` in generated Postgres migrations.
- Relation fields should use `@relation(...)` and are treated as virtual (not emitted as table columns).
- `@relation("belongsTo", ..., "<fk>")` triggers generated FK index SQL on the owning table.

## PHPX Usage Example
```php
import { connect, query_one, close } from 'db/postgres';
import { result_is_ok } from 'core/result';

$conn = connect({
  host: '127.0.0.1',
  port: 5432,
  database: 'app_db',
  user: 'postgres',
  password: 'secret'
});

if (!result_is_ok($conn)) {
  echo $conn->error;
  return;
}

$row = query_one($conn->value, 'select 1 as ok');
if (result_is_ok($row)) {
  echo 'ok';
}

close($conn->value);
```

## Linkhash Integration Pattern
- Linkhash should import database modules directly from project `php_modules`.
- Use `db` for shared primitives and `db/postgres` for PostgreSQL ergonomics.
- Return `Result` for connect/query_one style calls, and arrays for list-returning queries.

## Pooling/Reuse
- Host keeps process-local connection state keyed by normalized config.
- `open` reuses existing handle when the same key is already active.
- v1 is intentionally simple to validate UX and runtime behavior quickly.

## Why This Is Loosely Coupled
- No `postgres_*` host op surface.
- Host op is action-based and driver-keyed.
- Driver-specific behavior remains inside driver package and host action handlers.
- Same contract can be extended for `sqlite` and `mysql`.

## Next Steps
1. Add statement prepare/cache and richer type decoding.
2. Add metrics/introspection for active handles and query timings. ✅
3. Expand Linkhash write-path repository coverage using direct `db/*` modules.
