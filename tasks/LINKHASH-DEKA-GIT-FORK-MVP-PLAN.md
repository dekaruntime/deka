# Linkhash Deka-Git Fork MVP Plan

Goal: fork `deka-git` into Linkhash and customize it as the canonical Git/package backend for PHPX/Deka, with no wrapper layer.

## Scope
- Fork lives at `linkhash/rust/deka-git`.
- Move off blockchain JWT/ledger auth.
- Use standard user identity + SSH key registration model (ed25519 public keys from `~/.ssh`).
- Keep Smart HTTP Git fully operational.
- Provide package install endpoint surface for `deka install` integration.

## Task Tracker
- [x] 1. Repository hygiene and baseline build
- [x] 2. Replace blockchain auth with Linkhash token auth
- [x] 3. Add SSH public key storage + key management API
- [x] 4. Migrate repo ownership paths from network/address to username/repo
- [x] 5. Remove deploy/JWT coupling from Git push path
- [x] 6. Add package metadata/artifact endpoints for installer consumption
- [x] 7. Wire Deka installer default remote to Linkhash and run test download
- [x] 8. End-to-end runbook + docs for push + install flow

## Execution Rules
- One commit per completed task in `linkhash/rust/deka-git` (and in `mvp` for tracker/docs changes).
- Update this file after each task with:
  - date/time
  - key decisions
  - commit hash
  - validation commands

## Decisions Log
- 2026-02-17: Use Smart HTTP + token auth for Git transport first; SSH keys are canonical identity keys and API-managed in this MVP pass.
- 2026-02-17: Keep package fetch protocol simple (JSON metadata + tarball/blob endpoint), optimize later.
- 2026-02-17: Keep repo ownership locked to token identity for MVP (`/:owner/:repo` requires `owner == auth.username`).

## Progress Log
- 2026-02-17 Task 1-5 complete in fork commit `c2f7c5b`
  - Replaced blockchain JWT + ledger verification with local token auth (`Authorization: Bearer` and Git Basic user:token).
  - Added bootstrap identity creation on startup (`bootstrap_username` + `bootstrap_token`).
  - Added SSH key schema + APIs (`GET/POST/DELETE /api/user/ssh-keys`) with ed25519 validation + SHA256 fingerprinting.
  - Migrated repo storage semantics from `{network}/{address}/{repo}` to `{owner}/{repo}`.
  - Removed deploy trigger coupling from push path.
  - Validation:
    - `cargo check` (pass)
    - `cargo test` (pass)
- 2026-02-17 Task 6 complete in fork commit `62d3951`
  - Added package release table + indexes in DB migrations.
  - Added authenticated publish endpoint: `POST /api/packages/publish`.
  - Added public metadata + download endpoints:
    - `GET /api/packages/:name`
    - `GET /api/packages/:name/:version`
    - `GET /api/packages/:name/:version/download`
  - Download endpoint builds tarballs from git refs using `git archive`.
  - Validation:
    - `cargo check` (pass)
    - `cargo test` (pass)
- 2026-02-17 Task 7 complete in commits `1c1cb43` (fork) + `28a9738` (mvp)
  - `pm` now supports PHP ecosystem installation from Linkhash package endpoints.
  - Default Linkhash registry base URL is now `http://localhost:8508` (override: `LINKHASH_REGISTRY_URL`).
  - `deka publish` default registry URL updated to `http://localhost:8508`.
  - Live test executed end-to-end:
    - pushed git repo to forked `deka-git`
    - published `stdlib/core@0.1.0`
    - ran `deka install --ecosystem php --spec stdlib/core@0.1.0`
    - verified files in `php_modules/stdlib/core/*` and lock entry in `deka.lock`.
- 2026-02-17 Task 8 complete in commits `41243ff` (fork) + `ef8d6ba` (mvp)
  - Updated fork README with current auth model, API contract, and publish/install sequence.
  - Added `docs/phpx/linkhash-package-runbook.md` with exact end-to-end commands.
