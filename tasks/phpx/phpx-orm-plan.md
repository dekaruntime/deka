# PHPX ORM Plan

## Goals

- [x] Use PHPX `struct` models as the schema source of truth.
- [x] Avoid a separate Prisma-like schema DSL.
- [x] Generate app-owned ORM client files into project `db/` (not `php_modules/`).
- [x] Support Drizzle-style query builder DX.
- [x] Support first-class IDE feedback (parser, validator, LSP).

## Canonical Flow

- [x] User defines models in `types/index.phpx` (or another file).
- [x] User runs `deka db generate types/index.phpx` or `deka db generate types/`.
- [x] CLI resolves/validates model input and generates initial `db/*`.
- [x] App imports generated client via `@/db`.
- [x] User applies migrations with `deka db migrate`.
- [x] User inspects generation status with `deka db info` (initial implementation).

## Generated Layout

- [x] `db/index.phpx` public client entrypoint (initial).
- [x] `db/client.phpx` generated client scaffold.
- [x] `db/meta.phpx` generated model metadata (initial).
- [x] `db/migrations/*.sql` migration files (initial `0001_init.sql` generation).
- [x] `db/.generated/*` internal generated helpers (initial `schema.json`).
- [x] `db/_state.json` generator state (initial).
- [x] Generated PHPX files include `AUTO-GENERATED` warning header.

## CLI Commands

- [x] `deka db generate [path]` implemented for initial artifact generation.
- [x] `deka db migrate` implemented for initial Postgres SQL migration apply.
- [x] `deka db info` implemented for generation + migration status summary.
- [x] `deka db flush` (dev-only) command scaffold added.
- [x] Implement generation/migration/info/flush internals (initial Postgres + filesystem state implementation).

## Generate Path Resolution Contract

- [x] File input uses exact file.
- [x] Directory input resolves `<dir>/index.phpx`.
- [x] Missing input defaults to `types/index.phpx` when present.
- [x] Invalid path yields actionable error with attempted resolution.
- [x] Prefer `.phpx` for model entry.

## Language + Tooling

- [x] Add struct field annotations (`@id`, `@unique`, etc.) in PHPX parser.
- [x] Store annotations in AST as metadata (not runtime object lowering).
- [x] Add annotation validation rules (unknown, duplicate, target mismatch, bad args).
- [x] Add canonical relation annotation `@relation("hasMany|belongsTo|hasOne", "Model", "foreignKey")`.
- [x] Validate relation annotation args and shape constraints (`hasMany` requires `array<...>` field type).
- [x] Add LSP completions/signature help/hover for annotations.
- [x] Add LSP diagnostics for annotation errors (via typechecker diagnostics pipeline).
- [x] Add `@/` alias support in typechecker and LSP.
- [x] Add `@/` alias support in runtime resolver and CLI.

## Drizzle-Style Client DX

- [x] `db.insert(Model)->values(...)->returning()->one()`
- [x] `db.select()->from(Model)->where(...)->many()`
- [x] `db.update(Model)->set(...)->where(...)->returning()->many()`
- [x] `db.delete(Model)->where(...)->exec()`
- [x] Generated client includes runnable non-fluent helpers: `selectMany`, `selectOne`, `insertOne`, `updateWhere`, `deleteWhere`, `transaction`.
- [x] Predicates/helpers scaffolded in generated client: `eq`, `and`, `or` (+ shape placeholders for richer query builder).
- [x] Predicates/helpers: `asc`, `desc`, `limit`, `offset`.
- [x] Eager-loading support via `db.select()->from(Model)->include('relation')->many()` and low-level `$includes` argument in `selectMany/selectOne`.
- [x] Transaction API (non-fluent helper in generated client).
- [x] Bound client supports fluent API via `createClient($meta, $handle)` and rebinding via `withHandle($handle)`.
- [x] Generated client exposes relation loader helper: `loadRelation($model, $row, $field)` with `hasMany|belongsTo|hasOne` semantics based on `db/.generated/schema.json` relation metadata.

## Relation + Storage Rules (Current)

- [x] `@relation(...)` fields are virtual relation metadata, not physical columns.
- [x] `@relation("belongsTo", ..., "<fk>")` auto-generates an index for the foreign key on the owning table.
- [x] `array<T>` fields without `@relation(...)` are persisted as `JSONB` in Postgres migrations.
- [x] `array<Struct>` should use explicit `@relation(...)`; otherwise it is treated as data and stored as `JSONB`.
- [x] Relation model argument is validated against field type (`array<Post>` must use `"Post"` in `hasMany`).
- [x] `belongsTo/hasOne` validates that `foreignKey` exists on the owning struct.

## Rollout

- [x] Phase 1: CLI command scaffolding (`db generate/migrate/info/flush`) and path resolution.
- [x] Phase 2: Annotation parser/AST support + parser tests.
- [x] Phase 3: Validation + LSP support for annotations and model relations.
- [x] Phase 4: ORM IR + Postgres migration generator.
- [x] Phase 5: Generated client implementation + `@/db` import path.
- [x] Phase 6: Linkhash migration to generated client.
- [x] Phase 7: Test hardening across parser/validation/generator/CLI/runtime.
  - [x] Parser/typechecker coverage for `@relation(...)`.
  - [x] LSP completion/hover coverage for relation annotations.
  - [x] Generator coverage for relation migration rules + schema metadata.
  - [x] CLI generation/migration state coverage in release tests.
- [x] Runtime e2e DB test in local opt-in harness (`scripts/test-phpx-db-e2e.sh`).
- [x] Runtime e2e DB test in CI (`.github/workflows/phpx-tooling.yml`).
- [x] Host DB stats include statement-cache metrics (`statement_cache_entries`, `statement_cache_hits`, `statement_cache_misses`).

## Acceptance Criteria

- [x] `deka db generate types/` resolves and validates `types/index.phpx`.
- [x] Generated client imports via `@/db` in app code.
- [x] `deka db migrate` applies pending migrations and records state.
- [x] LSP shows clear diagnostics/completions for model annotations.
- [x] Linkhash reads/writes via generated Postgres client in primary flows.
