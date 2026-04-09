# Registry Auth + Context Tasks

This file is the umbrella view.

Execution is now split into two explicit tracks:

1. Runtime/platform-first track: `tasks/REGISTRY-RUNTIME-TASKS.md`
2. Linkhash app integration track: `tasks/LINKHASH-IMPLEMENTATION-TASKS.md`

Current direction: finish runtime track first, then implement linkhash track.

## Goals

- [x] Use Bluesky OAuth as the primary user identity for linkhash.
- [x] Map Bluesky handle to package namespace (`@handle/package`).
- [x] Introduce canonical immutable package IDs (`linkha.sh/lh_<sha256(...)>`).
- [x] Issue scoped publish tokens for CLI use (`read`, `read:write`, `read:write:delete`).
- [x] Provide first-class PHPX auth primitives (`crypto`, `jwt`, `cookies`, `auth`).
- [x] Add framework-level request context providers (React-like provider model, server-first).

## Artifact Storage (Local now, R2-ready)

- [x] Keep metadata/auth state in Postgres only.
- [x] Store package tarball bytes in pluggable artifact backend.
- [x] Implement `ARTIFACT_BACKEND=local|r2` config.
- [x] Implement local backend as default (`ARTIFACT_LOCAL_ROOT`).
- [x] Add R2 backend adapter interface and wire R2 config/envs.
- [x] Persist artifact metadata in Postgres (`artifact_key`, `size_bytes`, `sha256`, `backend`, `mime`).
- [x] Ensure install/download endpoints resolve artifact by key through active backend.

## Canonical Package Identity

- [x] Define canonical hash input contract:
- [x] `lower(handle) + "/" + lower(package) + ":" + version + ":" + lock_hash`
- [x] Compute `canonical_id = "lh_" + sha256(canonical_input)`.
- [x] Persist canonical identity per published version (unique).
- [x] Support both URL forms:
- [x] Human alias `linkha.sh/@handle/package[@version]`
- [x] Canonical `linkha.sh/lh_<sha256...>`
- [x] Always resolve aliases to canonical identity in responses.

## Auth Primitives (Runtime + PHPX modules)

- [x] `crypto` module
- [x] random bytes/id helpers
- [x] SHA-256 + HMAC-SHA256 helpers
- [x] `jwt` module
- [x] sign + verify JWT (HS256 first)
- [x] validate `exp`, `nbf`, `iat`, `iss`, `aud`
- [x] `cookies` module
- [x] ergonomic read from `$_COOKIE`
- [x] safe `Set-Cookie` builder (`HttpOnly`, `Secure`, `SameSite`, `Path`, `Domain`, `Max-Age`)

### JWT blocker note

- [x] Fix `encoding/json` object decode stability (unblocked JWT claim decode/validation in PHPX runtime).

## Linkhash OAuth + Token Flow

- [x] `/auth/login` redirect to Bluesky OAuth
- [x] `/auth/callback` code exchange + identity fetch
- [x] local account/org mapping from Bluesky identity (dev-login path in place, Bluesky callback wiring pending)
- [x] session cookie issuance and rotation
- [x] PAT management (create/list/revoke) with scope + expiry
- [x] token hashing-at-rest + last-used metadata

## Registry Permission Model

- [x] anonymous `read` for package metadata/install endpoints
- [x] `read:write` required for publish/update
- [x] `read:write:delete` required for delete/yank/owner mutation
- [x] unified scope checker reused by HTTP handlers and CLI auth validation
- [x] add package visibility model (`public`, `private`)
- [x] enforce private-package read access (`publisher`, `maintainer`) or scoped token

### Role Policy (Locked)

- `publisher` is the owner role (full control of org/package settings and release lifecycle)
- `maintainer` has edit/publish access but cannot transfer ownership or delete org
- `public` is read/view only access for public packages
- package visibility:
- `public` packages readable by anyone
- `private` packages readable only by org `publisher`/`maintainer` or tokens with explicit private-read grant

## Framework Context Provider Model (Server-first)

- [x] per-request context store in runtime/framework layer
- [x] provider composition at route/layout boundary
- [x] `auth()` helper for current request auth state
- [x] `requireAuth()` guard helper
- [x] `requireScope(...)` guard helper
- [x] `useContext(...)` read helper for component render
- [x] context re-evaluation on partial navigation requests
- [x] minimal safe auth snapshot for hydration (no secrets/tokens)

## CLI + Registry Integration

- [x] `deka publish` uses `LINKHASH_TOKEN` PAT
- [x] `deka install` resolves PHPX ecosystem packages from linkhash registry
- [x] write PHP lock entries under shared `deka.lock` (`php` section)
- [x] docs for `.env` token setup and scope requirements

## Hardening Pass (After Bulk Auth Build)

- [x] Rotate session ID on login/privilege changes (fixation protection).
- [x] Add session idle timeout + absolute timeout enforcement.
- [x] Enforce PKCE + OAuth state + nonce checks.
- [x] Add strict redirect URI allowlist.
- [x] Add CSRF protection for state-changing browser endpoints.

Notes:
- State-changing endpoints enforce `POST` and validate `X-CSRF-Token` against server-side session CSRF hash.
- [x] One-time PAT reveal + hashed-at-rest token storage.
- [x] Add PAT last-used metadata + bulk revoke.
- [x] Add org role model (`publisher`, `maintainer`, `public`) with private package support.
- [x] Add reserved namespace policy.
- [x] Add auth/publish rate limiting and abuse guardrails.
- [x] Add immutable audit trail for auth + publish actions.

### Reserved Namespace Defaults (Locked)

- deny org/handle registration for:
- `deka`, `linkhash`, `admin`, `support`, `system`, `root`, `security`, `api`, `auth`, `www`, `assets`, `cdn`, `status`, `ops`, `infra`, `help`, `docs`, `mail`, `postmaster`, `abuse`
- deny package names for:
- `core`, `runtime`, `php`, `phpx`, `stdlib`, `modules`, `internal`

### Rate Limit Defaults (Locked)

- auth start/callback:
- `GET /api/auth/login`, `GET /api/auth/callback`: `30/min/IP`
- token mutation:
- `POST /api/auth/token/create`, `POST /api/auth/token/revoke`, `POST /api/auth/token/revoke-all`: `10/min/user`, `30/min/IP`
- publish mutation:
- publish/update/delete endpoints: `5/min/user`, `20/min/IP`

## Acceptance Criteria

- [x] User logs in via Bluesky and gets a valid session in linkhash UI.
- [x] User can create PAT with explicit scopes and expiry.
- [x] `deka publish` succeeds with valid PAT and fails with clear scope errors otherwise.
- [x] `deka install` pulls published PHPX package from linkhash.
- [x] Published package versions can be addressed via canonical hash ID and alias routes.
- [x] Local artifact backend works in dev without R2.
- [x] Switching to R2 backend does not require changing publish/install API contracts.
- [x] Route-level provider guards correctly allow/deny access without client-side trust.
