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

`deka.gg/api/registry/<name>.json` and `/packages` are built from **R2**.
Website deploy runs `bun run probe:r2`, which HEADs candidate versions from
`data/registry.ts` against the public bucket and writes
`data/r2-artifacts.json`. Only objects that exist are listed. GitHub
`main`, docs-lock dates, and download counters are not the source of
truth. A version that is only a git tag does not appear until the tarball
is on R2 **and** the site has been redeployed.

## Package release workflow

Each stdlib repo has `.github/workflows/release.yml`. It runs on a pushed
`v*` tag, or `workflow_dispatch` with a version input (not on merge to
`main`).

**Never `ubuntu-latest`. Never any GitHub-hosted runner.** Org self-hosted
only: `runs-on: [self-hosted, macOS, ARM64]`. GitHub-hosted minutes are
not used for stdlib (or runtime) CI. The runners must be assigned at org
level so every `dekaruntime/<pkg>` repo can see them; a runner registered
only on `dekaruntime/deka` will not pick up `bytes` jobs.

1. Checkout the tagged (or dispatched) commit.
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

Confirm the object with a ranged GET (plain `HEAD` on this bucket often
hangs or 403s):

```bash
curl -sS -r 0-0 -D - -o /dev/null \
  https://pub-6d81db17678348abba85f93fde4b4400.r2.dev/<name>/0.2.0/<name>-0.2.0.tgz
```

`206` means the tarball is up. That is **not** enough for `deka add` until
the website has probed R2 and redeployed.

### 4. Website deploy probes R2

`data/registry.ts` is only a candidate list. After the tarball exists:

1. If the version is not already in `registry.ts`, append it (keep older
   versions).
2. Deploy `dekaruntime/website`. The deploy job runs `bun run probe:r2`
   then regenerates `public/api/registry/*.json` from objects that exist.

Until that deploy, `deka add <name>` still installs the previous latest
that is actually on R2. Do not hand-edit `/packages` dates or download
counts — they are not R2 metadata.

### 5. Host catalog vs package tarball

`deka.json` `"host": { "kinds": [...] }` is how the isolate grants
`bridge` ops — but **only for official `@deka/*` packages**. The enforcement
reality is:

- An **application** manifest (any non-`@deka/*` name) that sets `host.kinds`
  is a **hard load error** (RFD 27): the project refuses to boot with
  "declares host.kinds, but only @deka/* packages may self-declare bridge
  kinds". Apps acquire bridge authority only from the grant table.
- A **dependency** manifest's `host.kinds` is untrusted and ignored — copying
  the field into a user package buys nothing. Dependency grants come only
  from the published **grant table** (`{ name, version, digest, kinds }`),
  looked up by the dependency's lockfile-pinned `fsGraph` digest in
  `deka.lock`. Until the signed-index plumbing lands, the table is supplied
  explicitly (`PoolConfig.host_grants` or the `DEKA_HOST_GRANTS` environment
  variable).
- The runtime catalog is generated from
  `runtime_core::host_bridge` (`js_catalog_json()`) — the single source of
  truth (deka#620), injected into each isolate at bootstrap. The **CLI** must
  catalog the same ops.

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

Grant mismatches fail at `deka run` (per-call: `Result.Err` with
`HostGrantDenied` / `PermissionDenied`, never a throw) or at load time
("package '…' is not granted any host kinds but contains bridge calls").
Runtime tags are `PUBLISH.md`, not this file.

## What does **not** publish a package

- Merging the package PR
- Bumping `deka.json` without a matching `v*` tag
- Tagging a version already listed as latest on the registry
- A runtime (`dekaruntime/deka`) release
- Pushing to a PR branch

`bytes` was the worked example: on 2026-08-22 `main` was DekaScript
`0.2.0` while the registry still listed `0.1.1` and R2 had no
`bytes-0.2.0.tgz`. It has since been released properly and is now the
**reference package** — copy its shape (`deka.json`, `index.ds`,
`tests/`, `.gitignore`, `.github/workflows/release.yml`).

Hats `tests/packages/` stay red until steps 3 and 4 both happen. Step 4
is the one that gets skipped: a `206` from R2 looks like success, and
`deka add` silently keeps installing the previous latest.

## Verifying a repo that has never released

`workflow_dispatch` exists so a release can be re-cut without burning a
version. It is also how you prove a *new* package repo can actually
release before you tag anything real:

```bash
gh workflow run release.yml --repo dekaruntime/<name> -f version=0.0.0-wiring-check
gh run list --repo dekaruntime/<name> --limit 1
curl -sS -r 0-0 -o /dev/null -w '%{http_code}\n' \
  https://pub-6d81db17678348abba85f93fde4b4400.r2.dev/<name>/0.0.0-wiring-check/<name>-0.0.0-wiring-check.tgz
```

A throwaway version is inert — `probe:r2` only HEADs candidates listed in
`data/registry.ts`, so a version that is not listed never appears on the
index. This exercises the whole path: org runner assignment, the
Cloudflare secrets, and the R2 write.

Worth doing because runner visibility is per-repo. A repo with **no
workflow runs at all** has never proven it can see the org self-hosted
runners, and that failure only shows up when you are trying to cut a real
release.

## Testsuite

Hats fixtures declare `"packages": ["crypto"]` (etc.) and `deka add`
from the index during dump. They stay red until the index serves the
`.ds` tarball. Do not vendor `index.ds` into a fixture to turn a cell
green.

Dump uses the **published** CLI unless `DEKA_NATIVE` is set. Pair a
local CLI with `DEKA_WASM` from the same commit (`PUBLISH.md`).
