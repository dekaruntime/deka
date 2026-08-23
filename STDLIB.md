# Publishing Deka stdlib packages

Runtime releases (`PUBLISH.md`) ship the CLI and WASM. Stdlib modules
(`@deka/crypto`, `@deka/fs`, …) are separate git repos and a separate
pipeline. Merging a package PR does **not** put it on the index.
`deka add crypto` never reads GitHub `main`; it reads the registry and a
versioned R2 tarball.

## What `deka add` actually does

For `@deka/<name>`:

1. GET `https://deka.gg/api/registry/<name>.json`
2. If the consumer asked for `latest` / `*`, pick the highest semver in
   `versions`. A pinned version is used as-is.
3. Download
   `https://pub-6d81db17678348abba85f93fde4b4400.r2.dev/<name>/<version>/<name>-<version>.tgz`
4. Extract into the consumer's `php_modules/` (legacy name; `ds_modules/`
   is equivalent) and record the digest in `deka.lock`.

Those registry JSON files are **static** on `dekaruntime/website`
(`data/registry.ts` → `public/api/registry/*.json`). They are not
generated from GitHub tags.

## Package release workflow

Each stdlib repo has `.github/workflows/release.yml`. It runs only on a
pushed `v*` tag (not on merge to `main`):

1. Checkout the tagged commit.
2. `tar` the repo (`--exclude='.git'`).
3. Upload to R2 key
   `deka-stdlib/<name>/<version>/<name>-<version>.tgz`
   via Wrangler (`cloudflare/wrangler-action`).

`version` is the tag without the leading `v`. The tarball is the git
tree, not a filtered npm pack. Do not commit `ds_modules/`, `deka.lock`,
or other install leftovers; they are gitignored for this reason.
Vendored `php_modules/` inside a tarball makes `deka add` refuse the
package.

There is no GitHub Release for these tags. The artifact is the R2
object.

## Cutting a package release

Do this **after** the code is on `main`.

### 1. Version must be newer than the index

Check the live registry:

```bash
curl -sS https://deka.gg/api/registry/<name>.json
```

Set `"version"` in `deka.json` to a semver **strictly greater** than the
highest listed version. Tagging `v0.1.1` when `0.1.1` is already PHPX on
the index leaves `deka add` on PHPX forever: latest still wins.

The DekaScript rewrites of the original PHPX modules are `0.2.0`.

If the package depends on other `@deka/*` modules, those dependency
versions in `deka.json` must already be published (tarball **and**
registry) before anyone can `deka add` this package. Publish leaves
before dependents:

- `@deka/crypto` before `@deka/jwt`
- `@deka/tcp` and `@deka/tls` before `@deka/http`

### 2. Land the version on `main`

Bump `deka.json` on the PR (not in a follow-up on `main` if you can
avoid it), merge, then tag that merge.

### 3. Tag from `main`

```bash
git checkout main
git pull origin main
# deka.json "version" is 0.2.0
git tag -a v0.2.0 -m "release @deka/<name> v0.2.0"
git push origin v0.2.0
```

Use an annotated tag. Watch the run:

```bash
gh run list --repo dekaruntime/<name> --workflow=release.yml --limit 3
gh run watch <RUN_ID> --repo dekaruntime/<name> --exit-status
```

Confirm the object:

```bash
curl -sSI https://pub-6d81db17678348abba85f93fde4b4400.r2.dev/<name>/0.2.0/<name>-0.2.0.tgz
```

A `200` means the tarball is up. That is **not** enough for `deka add`.

### 4. Point the registry at the new version

In `dekaruntime/website`:

1. Append the version to `data/registry.ts` for that package
   (keep older versions in the array).
2. Run `bun scripts/generate-registry-static.ts`.
3. Merge and deploy `deka.gg`.

Until this ships, `deka add <name>` still installs the previous latest
(PHPX `0.1.1` / `0.1.2`). The tarball can sit on R2 unused.

### 5. Host catalog vs package tarball

`deka.json` `"host": { "kinds": [...] }` is how the isolate grants
`bridge` ops. The **CLI** must also catalog those ops.

| Package | Host kind | Notes |
|---|---|---|
| `crypto` | `crypto` | In 0.27.0 |
| `fs` | `fs` | `read_file` in 0.27.0; `write_file` / `read_dir` / `mkdirs` need a runtime newer than 0.27.0 |
| `tcp` | `net` | In 0.27.0 |
| `tls` | `tls` | `tls.upgrade` in 0.27.0 |
| `time` | `time` | `now()` is `Date.now` (no host op); `sleep_ms` needs a runtime newer than 0.27.0 |
| `json` | none | JS `JSON.parse` / `stringify` |
| `jwt` | none | DS on `@deka/crypto` |
| `http` | none | DS on `@deka/tcp` + `@deka/tls` |
| `bytes` | none | |

Publishing a package that calls an uncatalogued op does not fail
`deka add`. It fails at `deka run`. Runtime tags are `PUBLISH.md`, not
this file.

## What does **not** publish a package

- Merging the package PR
- Bumping `deka.json` without a matching `v*` tag
- Tagging a version already listed as latest on the registry
- A runtime (`dekaruntime/deka`) release
- Pushing to a PR branch

`bytes` on 2026-08-22 is the example: `main` is DekaScript `0.2.0`, the
registry still lists `0.1.1`, R2 has no `bytes-0.2.0.tgz`. Hats
`tests/packages/` stay red until steps 3 and 4 both happen.

## Testsuite

Hats fixtures declare `"packages": ["crypto"]` (etc.) and `deka add`
from the index during dump. They stay red until the index serves the
`.ds` tarball. Do not vendor `index.ds` into a fixture to turn a cell
green.

Dump uses the **published** CLI unless `DEKA_NATIVE` is set. Pair a
local CLI with `DEKA_WASM` from the same commit (`PUBLISH.md`).
