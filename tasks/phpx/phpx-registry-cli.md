# PHPX Registry CLI Auth

This doc covers local CLI auth for PHPX publish/install workflows.

## Environment

- `LINKHASH_REGISTRY_URL`
  - Base registry URL used by CLI auth/publish/install paths.
  - Default: `http://localhost:8530`
- `LINKHASH_TOKEN`
  - Personal access token used by `deka publish` when `--token` is not passed.

## Publish

`deka publish` accepts explicit CLI args and falls back to `LINKHASH_TOKEN`.

Example:

```bash
LINKHASH_REGISTRY_URL=http://localhost:8530 \
LINKHASH_TOKEN=lh_pat_xxx \
deka publish \
  --org-id org_123 \
  --name demo \
  --version 0.1.0 \
  --file ./dist/demo.tgz
```

Required scope for publish/update: `read:write`.

## Install (PHPX ecosystem)

`deka install` resolves PHP packages from linkhash-compatible endpoints when using
PHP package specs (for example `@org/name` or `@org/name@version`).

Example:

```bash
LINKHASH_REGISTRY_URL=http://localhost:8530 \
deka install --ecosystem php --spec @samifouad/demo@0.1.0
```

## Lockfile

- CLI writes a shared `deka.lock` at project root.
- PHP package entries are written under the `php` section in that lockfile.
- Node package entries remain under the `node` section.
