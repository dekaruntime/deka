---
category: "linkhash-package-runbook"
version: "latest"
---
# Linkhash Package Runbook (MVP)

This runbook covers the Git push + package publish + `deka install` flow for Linkhash-backed PHP packages.

## Defaults

- Linkhash package registry default: `http://localhost:8508`
- Override with `LINKHASH_REGISTRY_URL`.
- `deka install --ecosystem php` uses Linkhash package endpoints.

## 1) Start Linkhash gild-vcs fork

Provide `config.toml` (example):

```toml
port = 8508
database_url = "postgres://deka:deka_dev_password@localhost:5434/deka"
repos_path = "./repos"
bootstrap_username = "linkhash-admin"
bootstrap_token = "test-token"
```

Start service:

```bash
cd linkhash/rust/gild-vcs
cargo run
```

## 2) Create and push a package source repo

Create remote repo:

```bash
curl -X POST http://localhost:8508/api/repos/stdlib \
  -H 'Authorization: Bearer test-token'
```

Push source:

```bash
git remote add origin http://linkhash-admin:test-token@localhost:8508/linkhash-admin/stdlib.git
git push -u origin main
```

## 3) Publish a package release

```bash
curl -X POST http://localhost:8508/api/packages/publish \
  -H 'Authorization: Bearer test-token' \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "stdlib/core",
    "version": "0.1.0",
    "repo": "stdlib",
    "git_ref": "main"
  }'
```

## 4) Install with deka

```bash
LINKHASH_REGISTRY_URL=http://localhost:8508 \
  deka install --ecosystem php --spec stdlib/core@0.1.0
```

Expected outputs:

- `php_modules/stdlib/core/*` exists in the project.
- `deka.lock` has an entry under `php.packages["stdlib/core"]`.

## Notes

- Current Git transport is Smart HTTP with token auth.
- SSH public key endpoints exist for identity/key management.
- Git/Package ownership is tied to authenticated username.
