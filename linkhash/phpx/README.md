# linkha.sh - PHPX Package Registry

A modern package registry for PHPX, built with PHPX itself.

## Features

- 🚀 Built with PHPX - showcasing the language's capabilities
- 📦 NPM-like package management for PHPX code
- 🔐 Bluesky OAuth authentication
- 🗄️ Postgres-backed package catalog
- 🧪 Runtime Lab route to probe Postgres, MySQL, SQLite, and fs/bytes live
- ⚡ Partial navigation with `component/dom` `Link` + `Hydration`
- 🎨 Tailwind CSS for beautiful UI
- 🐳 Docker-ready for easy deployment

## Quick Start

```bash
# Start the development server
deka serve main.phpx

# Or with Docker
docker-compose up
```

## Architecture

This registry is built entirely in PHPX and demonstrates:

- Type-safe API endpoints
- JSX components for frontend
- Postgres integration via the generated ORM client (`deka db generate` + `deka db migrate`)
- Modern error handling with Result types
- Tailwind CSS styling

## Development

```bash
# Install dependencies (when we have a package manager!)
deka install

# Run tests
deka test

# Build for production
deka build
```

## Runtime Lab

- Page: `/runtime`
- API: `/api/runtime-checks`

This validates new PHPX runtime features in one place:

- `db/postgres`, `db/mysql`, `db/sqlite`
- `fs` + `bytes`
- `db.stats()` host metrics
- `component/dom` partial navigation

## Postgres Setup

```bash
# Ensure postgres container is running (example from local setup)
docker start linkhash-pg

# Connection settings
export DB_HOST=127.0.0.1
export DB_PORT=55432
export DB_NAME=linkhash_registry
export DB_USER=postgres
export DB_PASSWORD=postgres
export LINKHASH_LOG_BACKEND=db
# optional text log file via runtime error_log sink
# export LINKHASH_LOG_FILE=/tmp/linkhash.log

# Run
deka serve main.phpx
```

Schema + ORM notes:

- Models live in `phpx/models/Models.phpx`.
- Migrations are stored in `phpx/db/migrations/`.
- Generate ORM files with `deka db generate models/Models.phpx`.
- Apply migrations with `deka db migrate` (uses the `DB_*` env vars).

## Docker Deployment (Local)

Use Docker Compose to run Linkhash + Postgres together:

```bash
cd /Users/samifouad/Projects/deka/linkhash
docker compose up --build
```

Notes:

- App endpoint: `http://localhost:8530`
- Postgres endpoint: `localhost:55432` (`postgres` / `postgres`)
- Compose builds `deka` from local source (`../deka/deka`) into the app image

## Bluesky OAuth Setup

Linkhash OAuth is configured entirely via env vars and does not depend on `deka.gg`.

```bash
export LINKHASH_OAUTH_CLIENT_ID=your-client-id
export LINKHASH_OAUTH_CLIENT_SECRET=
export LINKHASH_OAUTH_CALLBACK=http://localhost:8530/api/auth/callback
export LINKHASH_OAUTH_AUTH_URL=https://bsky.social/oauth/authorize
export LINKHASH_OAUTH_TOKEN_URL=https://bsky.social/oauth/token
export LINKHASH_OAUTH_PROFILE_URL=https://bsky.social/xrpc/com.atproto.server.getSession
export LINKHASH_OAUTH_SCOPE="atproto transition:generic"
export LINKHASH_OAUTH_CALLBACK_ALLOWLIST=http://localhost:8530/api/auth/callback
```

Endpoints:

- `GET /api/auth/login` returns provider authorize URL + sets state/PKCE cookies
- `GET /api/auth/callback?code=...&state=...` exchanges code, fetches profile, creates local session + CSRF token
- `POST /api/auth/token/create` creates PAT (requires `X-CSRF-Token` for session auth)
- `POST /api/auth/token/revoke` revokes PAT (requires `X-CSRF-Token` for session auth)
- `POST /api/auth/token/revoke-all` revokes all PATs for current user (requires `X-CSRF-Token`)
- `GET /api/orgs/mine` lists org memberships for current user
- `POST /api/orgs/create` creates an org (`handle`, optional `visibility`) and assigns caller as `owner`
- `GET /api/orgs/members?orgId=...` lists org members (requires org access)
- `POST /api/orgs/members/upsert` sets member role (`publisher` or `maintainer`) (requires org manage access)
- `POST /api/orgs/members/remove` revokes member access (requires org manage access)
- `POST /api/packages/visibility?name=...&visibility=public|private` updates package visibility (requires org edit access)
- `GET /api/package?org=...&name=...` returns package metadata + version list (visibility aware)
- `GET /api/org?handle=...` returns org profile + package list (visibility aware)
- `GET /api/stats/packages` returns registry-level counts (packages, orgs, versions, downloads)
- `POST /api/publish` publishes a package version + artifact bytes (MVP uses inline base64 bytes)
- `GET /api/install?org=...&name=...&version=latest|x.y.z` resolves install metadata + download URL
- `GET /api/artifacts/:canonicalId` downloads artifact bytes (private packages require org access)
- `POST /api/repo/issues/create` creates an issue for the backing repo
- `POST /api/repo/issues/comment` comments on an issue
- `POST /api/repo/pulls/create` creates a pull request for the backing repo
- `POST /api/repo/pulls/comment` comments on a pull request

Logging:

- API requests include `X-Request-Id` for correlation
- Unknown API routes are logged as `api.not_found`
- Runtime warnings/shutdown errors are logged as `php.warning` / `php.shutdown`
- With `LINKHASH_LOG_BACKEND=db` (default), events are stored in `lh_event_log`
- `LINKHASH_LOG_FILE` is optional and appends JSON lines when runtime supports `error_log` file sink

SSR routes:

- `/playground` package inspector (resolve version + inspect install/artifact metadata)
- `/{org}/{repo}` repository/package landing page, for example `/deka/json`
- `/package/{org}/{name}` legacy package detail URL
- `/@{owner}/{repo}/{ref}` preview resolver page (maps ref to commit for Adwa preview)
- `/fork/result` post-fork page with clone instructions + editor links
- `/org/{handle}` organization profile page

Package detail docs:

- Package pages now show version-aware API docs symbols when `linkhash-git` docs APIs are reachable.
- Configure the backend URL with `LINKHASH_GIT_API_URL` (default `http://localhost:8508`).
- Package pages use `tree`/`blob` APIs for file explorer + source preview when available.
- Repository/package indexes only list packages with `visibility=public`; private packages stay hidden from index cards even for authenticated viewers.
- Package pages now render issue/PR list/detail/create/comment panels against `linkhash-git` repo APIs.
- Set `LINKHASH_GIT_TOKEN` (or `LINKHASH_GIT_API_TOKEN`) so Linkhash can call authenticated repo APIs.
- Package pages include `Preview HEAD in Adwa` and fork controls.
- Set `LINKHASH_ADWA_BASE_URL` (default `http://localhost:8600`) for Adwa preview/editor links.

Role model:

- `owner`: full org management access
- `publisher`: org management + publish/edit
- `maintainer`: publish/edit only
- public users: read/view only

Visibility model:

- Anonymous users only see `public` packages
- Authenticated org members can also see matching `private` packages

Publish/install notes:

- `POST /api/publish` currently accepts query/body fields:
  - `orgId`, `name`, `version`, `lockHash`, `sha256`
  - optional integrity fields: `integrityAlgo`, `moduleGraphHash`, `fsGraphHash`
  - optional: `visibility`, `mime`, `mainFile`, `description`, `artifactBase64`
- Artifact storage supports two modes:
  - `LINKHASH_ARTIFACT_BACKEND=local` (default): Postgres inline bytes (`artifact_inline_b64`)
  - `LINKHASH_ARTIFACT_BACKEND=r2`: attempt R2 upload first, then automatically fall back to inline Postgres bytes
- `GET /api/install` increments package download counters and records a `downloads` row

R2 env vars (optional):

```bash
export LINKHASH_ARTIFACT_BACKEND=r2
export LINKHASH_R2_ACCOUNT_ID=...
export LINKHASH_R2_BUCKET=...
export LINKHASH_R2_ACCESS_KEY_ID=...
export LINKHASH_R2_SECRET_ACCESS_KEY=...
export LINKHASH_R2_REGION=auto
# optional override, otherwise {account}.r2.cloudflarestorage.com
export LINKHASH_R2_ENDPOINT=
```

Rate limits (default):

- Login/callback/dev-login: `30/min/IP`
- PAT mutate: `10/min/user` + `30/min/IP`
- Publish/org mutate: `5/min/user` + `20/min/IP`

Scope behavior:

- Scopes are cumulative:
  - `read` = read-only
  - `read:write` = read + write
  - `read:write:delete` = read + write + delete

## Database Modules (PHPX)

This project uses PHPX modules for database access:

- `php_modules/db` for generic DB primitives
- `php_modules/db/postgres` for Postgres ergonomics

Quick smoke test:

```bash
deka run __db_layer_test.phpx
```

Expected output without a running/valid DB:

```txt
connect_err
```
