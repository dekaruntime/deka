# PHPX Bridge Protobuf Migration Plan

## Goal
- Migrate PHPX host bridge transport from ad-hoc JSON payloads to Protobuf while keeping user-facing PHPX APIs stable (`db/*`, `fs`, `tcp`, `tls`).
- Keep bridge internals private (`__deka_*` not userland API).
- Preserve runtime execution model (no build step required for app code).

## Scope
- In scope: `db`, `fs`, `net` bridge kinds, Rust ops, JS host shim, PHPX core modules, tests/docs.
- Out of scope: changing public module function names/signatures during migration.

## Rollout Strategy
- Domain-by-domain cutover with fallback compatibility.
- First domain: `db`.
- Keep JSON fallback temporarily until parity tests pass, then remove.

## Protobuf Versioning Policy (Bridge v1)
- Request/response envelopes must include `schema_version`.
- New fields must be additive and optional-compatible.
- Existing field numbers are immutable after release.
- Removed fields must be reserved (field numbers and names).
- Unknown fields must be ignored by decoders.
- Breaking wire changes require `v2` message families and explicit runtime switch.
- Runtime behavior rule:
- `schema_version == 1`: serve with v1 handlers.
- unsupported version: return structured bridge error, no partial dispatch.

## Tasks

### Phase 1: Schema and Generation
- [x] 1. Define bridge envelope and `db` action messages in `.proto`.
- [x] 2. Add Rust protobuf codegen (build-time generation in `modules_php`).
- [x] 3. Add JS protobuf encode/decode support in `deka_php/php.js`.
- [x] 4. Document versioning policy for backward-compatible schema evolution.

### Phase 2: DB Bridge Migration
- [x] 5. Implement Protobuf decode/dispatch in `op_php_db_call`.
- [x] 6. Implement Protobuf encode responses for `open/query/query_one/exec/tx/close/stats`.
- [x] 7. Keep temporary JSON fallback path behind runtime feature flag.
- [x] 8. Add parity tests (`json` vs `protobuf`) for `postgres/mysql/sqlite`.

### Phase 3: FS Bridge Migration
- [x] 9. Add `fs` schema messages (`open/read/write/close/read_file/write_file`).
- [x] 10. Migrate `op_php_fs_call` to Protobuf transport.
- [x] 11. Add binary integrity tests for file bytes round-trip.

### Phase 4: Net Bridge Migration
- [x] 12. Add `net` schema messages (`connect/read/write/tls_upgrade/close/deadline`).
- [x] 13. Migrate `op_php_net_call` to Protobuf transport.
- [x] 14. Add TCP/TLS protocol sanity tests.

### Phase 5: Cleanup and Hardening
- [x] 15. Remove JSON bridge fallback paths.
- [x] 16. Add bridge fuzz/safety tests for malformed payloads.
- [x] 17. Add bridge metrics for decode/encode overhead and payload size.
- [x] 18. Update AGENTS/docs with mandatory validation test commands.

## Validation Gate Per Task
- Run `cargo check -p modules_php`.
- Run targeted runtime tests for affected bridge domain.
- Commit after passing checks.

## Mandatory Validation Commands
- `cargo check -p modules_php`
- `cargo test -p modules_php "proto_" -- --nocapture`
- If bridge runtime JS changed: `cargo check -p modules_php` and run one smoke server start (`deka serve <entry>.phpx`) before merge.
