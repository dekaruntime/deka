# Linkhash Registry API

Linkhash is the PHPX package registry used by Deka and Tana. It stores immutable
package releases, validates publish requests against security declarations, and
records API snapshots so consumers can install versioned modules instead of
copying source files.

The production registry is served by the Tana git server. Local development
usually runs the same API at `http://localhost:9418`.

## Authentication

All registry endpoints currently require bearer-token authentication.

```http
Authorization: Bearer <token>
```

Tokens are checked for package scopes:

- `packages:read` is required for package listing, release lookup, docs, tree,
  and blob reads.
- `packages:write` is required for preflight and publish.
- Publish also requires write ACL on the source repository. For `@tana/store`,
  the authenticated owner must be `tana` and the source repo must be `store`.

The Deka CLI reads registry auth from, in order:

- explicit flags such as `--token` and `--registry-url`
- the local `deka login` auth profile
- `LINKHASH_TOKEN` or `TANA_GIT_TOKEN`
- `LINKHASH_REGISTRY` or `TANA_GIT_SERVER`

## Package Identity

Published PHPX packages must use scoped names:

```text
@scope/name
```

The registry enforces identity at publish time:

- the package scope must match the authenticated owner
- the package name segment must match the source repo name
- `deka.json` must contain the same `name` and `version` as the publish request
- the publish ref must resolve to the release tag `v<version>`

Package names and scopes may contain ASCII letters, numbers, `_`, and `-`.
Versions must parse as SemVer.

## Package Manifest

Every release is backed by a `deka.json` manifest in the tagged source tree or
by an equivalent `manifest` object in the publish request.

Minimal manifest:

```json
{
  "name": "@tana/store",
  "version": "1.2.0",
  "description": "Tana storefront module"
}
```

Security-sensitive packages must declare the capabilities detected in source:

```json
{
  "name": "@tana/store",
  "version": "1.2.0",
  "deka.security": {
    "allow": {
      "dynamic": true,
      "run": ["checkout-worker"]
    }
  }
}
```

Current publish-time capability detection reports:

- `dynamic` for dynamic code execution patterns such as `eval(...)`,
  `new Function(...)`, or dynamic import patterns.
- `run` for process or shell execution patterns such as `Command::new(...)`,
  `shell_exec(...)`, `proc_open(...)`, or shell backticks.

Publish is rejected when detected capabilities are missing from
`deka.security.allow`.

## Version Semantics

Linkhash compares the API snapshot of a new release against the latest published
release for that package.

- Initial publish: any valid SemVer is accepted.
- Patch release: no public API signature changes; minimum is previous patch + 1.
- Minor release: public exports were added; minimum is previous minor + 1 with
  patch reset to `0`.
- Major release: public exports were removed or public signatures changed;
  minimum is previous major + 1 with minor and patch reset to `0`.

The requested version must be greater than the latest published version and must
be at least the minimum version required by the detected API change.

`latest` resolves to the highest SemVer release, not simply the most recently
inserted row. Exact release reads use an exact version string. The CLI-facing
install contract may request ranges such as `^1.0.0`; registries should resolve
those to the highest compatible release and return the same release shape as an
exact version lookup.

## Endpoints

Use the scoped package routes for package names like `@tana/store`; they avoid
URL-encoding ambiguity around `/` in package names.

### List Packages

```http
GET /api/packages
```

Required scope: `packages:read`

Response:

```json
{
  "packages": [
    {
      "name": "@tana/store",
      "versions": ["1.2.0", "1.1.0"],
      "latest": "1.2.0"
    }
  ]
}
```

### List Versions

```http
GET /api/scoped-packages/{scope}/{name}/versions
```

Example:

```http
GET /api/scoped-packages/tana/store/versions
```

Required scope: `packages:read`

Response:

```json
{
  "name": "@tana/store",
  "versions": ["1.2.0", "1.1.0"],
  "latest": "1.2.0"
}
```

### Get Latest Release

```http
GET /api/scoped-packages/{scope}/{name}/latest
```

Required scope: `packages:read`

Response:

```json
{
  "package_name": "@tana/store",
  "version": "1.2.0",
  "owner": "tana",
  "repo": "store",
  "git_ref": "v1.2.0",
  "description": "Tana storefront module",
  "manifest": "{\"name\":\"@tana/store\",\"version\":\"1.2.0\"}",
  "api_snapshot": "{\"exports\":{}}",
  "api_change_kind": "minor",
  "required_bump": "minor",
  "capability_metadata": "{\"detected\":[],\"declared\":[],\"missing\":[]}",
  "created_at": "2026-05-15T00:00:00Z"
}
```

### Get Exact Release

```http
GET /api/scoped-packages/{scope}/{name}/{version}
```

Required scope: `packages:read`

Returns the same release object as the latest endpoint.

### Resolve Version Range

```http
GET /api/scoped-packages/{scope}/{name}/resolve?range={range}
```

Required scope: `packages:read`

This is the CLI-facing resolution contract for range installs such as
`@tana/store@^1.0.0`. It should return the highest SemVer release satisfying
the requested range, using the same release object as the exact release
endpoint. `latest`, empty, and `*` resolve like the latest endpoint.

Current clients fall back to exact release lookup when the requested spec is an
exact SemVer. Registries should keep this route compatible when range
resolution is enabled server-side.

### Get API Docs

```http
GET /api/scoped-packages/{scope}/{name}/{version}/docs
```

Required scope: `packages:read`

Response:

```json
{
  "package_name": "@tana/store",
  "version": "1.2.0",
  "symbols": [
    {
      "symbol": "renderProductCard",
      "kind": "function",
      "signature": "function renderProductCard(product: Product): Component",
      "source": "src/product-card.phpx",
      "summary": "Render a storefront product card.",
      "description": null,
      "examples": []
    }
  ]
}
```

### Get Release Tree

```http
GET /api/scoped-packages/{scope}/{name}/{version}/tree
```

Required scope: `packages:read`

Response:

```json
{
  "package_name": "@tana/store",
  "version": "1.2.0",
  "git_ref": "v1.2.0",
  "entries": [
    {
      "mode": "100644",
      "kind": "blob",
      "object": "6f1ed002ab5595859014ebf0951522d9",
      "size": 2418,
      "path": "src/index.phpx"
    }
  ]
}
```

### Get Release Blob

```http
GET /api/scoped-packages/{scope}/{name}/{version}/blob?path={path}
```

Required scope: `packages:read`

Response:

```json
{
  "package_name": "@tana/store",
  "version": "1.2.0",
  "path": "src/index.phpx",
  "git_ref": "v1.2.0",
  "content": "<?phpx\nexport function renderStore() { ... }\n"
}
```

### Preflight Publish

```http
POST /api/packages/preflight
Content-Type: application/json
```

Required scope: `packages:write`

Request:

```json
{
  "name": "@tana/store",
  "version": "1.2.0",
  "repo": "store",
  "git_ref": "v1.2.0",
  "description": "Tana storefront module",
  "manifest": {
    "name": "@tana/store",
    "version": "1.2.0"
  }
}
```

`git_ref`, `description`, and `manifest` are optional. When `git_ref` is omitted,
the registry checks `HEAD`; production publish flows should use the release tag.
When `manifest` is omitted, the registry reads `deka.json` from the git ref.

Response:

```json
{
  "package_name": "@tana/store",
  "requested_version": "1.2.0",
  "previous_version": "1.1.0",
  "detected_change": "minor",
  "required_bump": "minor",
  "minimum_allowed_version": "1.2.0",
  "allowed": true,
  "reasons": ["added export `renderProductCard`"],
  "issues": [
    {
      "code": "API_ADDED_EXPORT",
      "severity": "info",
      "symbol": "renderProductCard",
      "message": "new public export was added",
      "old_source": null,
      "new_source": "src/product-card.phpx",
      "old_signature": null,
      "new_signature": "function renderProductCard(product: Product): Component"
    }
  ],
  "capabilities": {
    "detected": [],
    "declared": [],
    "missing": []
  }
}
```

### Publish Release

```http
POST /api/packages/publish
Content-Type: application/json
```

Required scope: `packages:write`

Request body is the same as preflight. Publish runs the same validations and
then inserts an immutable release record.

Success status: `201 Created`

Response:

```json
{
  "package_name": "@tana/store",
  "version": "1.2.0",
  "owner": "tana",
  "repo": "store",
  "git_ref": "v1.2.0",
  "description": "Tana storefront module",
  "manifest": "{\"name\":\"@tana/store\",\"version\":\"1.2.0\"}",
  "api_snapshot": "{\"exports\":{}}",
  "api_change_kind": "minor",
  "required_bump": "minor",
  "capability_metadata": "{\"detected\":[],\"declared\":[],\"missing\":[]}",
  "created_at": "2026-05-15T00:00:00Z"
}
```

## Error Shape

Errors are returned as JSON:

```json
{
  "error": "packages:read scope required"
}
```

Common statuses:

- `400 Bad Request`: invalid JSON, invalid package metadata, missing tag,
  undeclared capabilities, or semver gate failure.
- `401 Unauthorized`: missing or invalid bearer token.
- `403 Forbidden`: token lacks package scope or repo write ACL.
- `404 Not Found`: package release or blob path was not found.
- `500 Internal Server Error`: database or git backend failure.

## Compatibility Routes

The server also exposes flat routes:

```http
GET /api/packages/{name}/versions
GET /api/packages/{name}/{version}
GET /api/packages/{name}/latest
GET /api/packages/{name}/{version}/docs
GET /api/packages/{name}/{version}/tree
GET /api/packages/{name}/{version}/blob?path={path}
```

Use `/api/scoped-packages/{scope}/{name}/...` for scoped packages. Flat routes
are mainly useful for unscoped names and internal compatibility because
`@scope/name` contains a slash.

## CLI Flow

Typical publish:

```bash
deka publish \
  --name @tana/store \
  --pkg-version 1.2.0 \
  --repo store \
  --git-ref v1.2.0 \
  --registry-url http://localhost:9418 \
  --token "$TANA_GIT_TOKEN"
```

Typical install:

```bash
deka install @tana/store@latest
deka install @tana/store@1.2.0
```

Install resolves the release, downloads the package tree into
`php_modules/@tana/store/`, and records the installed version plus integrity
hashes in `deka.lock`.

Install clients must treat tree/blob paths as untrusted registry input. Absolute
paths, drive prefixes, empty components, `.`, and `..` are rejected before any
directory is created or blob is written.
