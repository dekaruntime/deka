# Publishing Deka stdlib packages

Runtime releases (`PUBLISH.md`) ship the CLI and WASM. Stdlib modules
(`@deka/crypto`, `@deka/fs`, …) are separate git repos and a separate
pipeline. **Publishing is automated (deka#511): the whole ritual is a PR
merged to `main` that bumps `"version"` in `deka.json`. Workflows do the
rest — tagging, packing, uploading, verifying, and updating the website
index. There is no manual step, and no hand-cut tag.**

## What `deka add` actually does

For `@deka/<name>`:

1. GET `https://deka.gg/api/registry/<name>.json`
2. If the consumer asked for `latest` / `*`, pick the highest semver in
   `versions`. A pinned version is used as-is.
3. Download
   `https://pub-6d81db17678348abba85f93fde4b4400.r2.dev/<name>/<version>/<name>-<version>.tgz`
4. Extract into the consumer's `ds_modules/` and record the digest in `deka.lock`.
5. Derive the package's RFD 27 host grant — the catalog kinds the runtime
   assigns to the `@deka/<name>` identity — and record
   `{ name, version, digest, kinds }` in `deka.grants.json` next to
   `deka.lock`, keyed by the fsGraph digest just pinned (deka#797).
   Commit `deka.grants.json` with `deka.lock`: a fresh checkout's
   `deka install` refuses to change it under `--locked`, and the runtime
   loader reads it after the explicit `PoolConfig.host_grants` /
   `DEKA_HOST_GRANTS` override channels. A package can hold only the kinds
   the catalog binds to its identity, and only for the digest the
   lockfile pins — never anything its own manifest declares.

`deka.gg/api/registry/<name>.json` and `/packages` are built from **R2**.
The website deploy runs `bun run probe:r2`, which **lists the R2 bucket
prefix** for each curated package name and ranged-GETs every listed
tarball before writing `data/r2-artifacts.json`. Only objects that exist
and are retrievable are listed. GitHub `main`, docs-lock dates, and
download counters are not the source of truth. A version that is only a
git tag does not appear until the tarball is on R2 **and** the site has
been redeployed.

## How publishing works

Every stdlib repo carries a thin `.github/workflows/release.yml` that
calls the shared reusable workflow in this repo,
`.github/workflows/stdlib-release.yml`. One workflow, ~19 repos, no
copies to drift (the old per-repo copies had already drifted: three
tarball-exclude variants, two different gates).

**Never `ubuntu-latest`. Never any GitHub-hosted runner.** Org self-hosted
only: `runs-on: [self-hosted, macOS, ARM64]`. GitHub-hosted minutes are
not used for stdlib (or runtime) CI. The runners must be assigned at org
level so every `dekaruntime/<pkg>` repo can see them; a runner registered
only on `dekaruntime/deka` will not pick up `bytes` jobs.

The workflow compiles no Rust: it downloads the prebuilt `deka` and `dsc`
CLIs (the current released compiler) and runs the package gate against
them, so the org sccache policy does not apply to it.

### Triggers

| Event | What happens |
|---|---|
| PR that touches `deka.json` | **Pre-merge gate**: install the package's declared `@deka/*` dependencies from the registry and typecheck the tree with `deka check --as-package .` plus `deka check tests/*.ds`. Must be green before merge. |
| Merge (push) to `main` | **Publish**, but only if the merge changed `version` in `deka.json`, the merge commit belongs to a merged PR that touched `deka.json`, and the version is not already on the registry. Otherwise no-op. |
| `workflow_dispatch` with `wiring_check_version` | Upload a throwaway tarball, verify it by ranged GET, delete it again. Proves a never-released repo can see the runners, the Cloudflare secrets, and the R2 write. |

### The publish run, in order

1. Read `version` from `deka.json` at the merge commit. If the merge did
   not change it, stop: merging a code-only PR publishes nothing.
2. Refuse direct pushes: if the merge commit has no associated PR, the
   run **fails with the reason** — a direct push to `main` that changes
   `version` does not publish, because the tag must be traceable to a
   reviewed, CI-green PR (Sami's scope note, deka#511). Land the bump via
   a PR instead.
3. No-op if the version is already on the registry. This is also what
   makes re-running a failed run safe.
4. Verify the package as an installed consumer (`deka check --as-package .`):
   every declared `@deka/*` dependency is resolved from the registry and
   the local tree is typechecked against it with the current compiler. A
   dependency that does not install or typecheck is a hard failure — this
   is the gate that would have caught `@deka/jwt` 0.3.1 pinning a
   `@deka/crypto` that did not parse. (The gate was unpassable for
   packages with dependencies until deka#512 was fixed by deka#513,
   released in 0.38.3; the workflow resolves the latest CLI, so it always
   runs against a compiler that has the fix.)
5. Create the annotated tag `v<version>` **from the merge commit**, via
   the GitHub API, unless it already exists. The tag is an output of the
   process, never a trigger; "tagged the same version twice" and "forgot
   the `v`" are impossible. There is no GitHub Release for these tags —
   the artifact is the R2 object.
6. Clean (`ds_modules/`, `php_modules/`, `.deka.json-backup-*` are
   removed, never tarred), pack the git tree with `tar`.
7. Upload to R2 key `deka-stdlib/<name>/<version>/<name>-<version>.tgz`
   via the S3 API.
8. **Verify the upload by ranged GET** (`curl -r 0-0`) against the public
   bucket URL — plain `HEAD` on this bucket hangs or 403s, and a green
   upload step is not evidence the object exists. Expect `206`; anything
   else fails the run. Then read the object back through the S3 API and
   compare SHA-256 with the local tarball.
9. Fire a `repository_dispatch` (`stdlib-published`) at
   `dekaruntime/website` with `{package, version}`. The website's deploy
   workflow listens for it and redeploys, so the index picks the new
   version up in minutes. If the dispatch token is missing the run warns
   and continues — the website's 2-hourly cron is the fallback.

Watch a run:

```bash
gh run list --repo dekaruntime/<name> --workflow=release.yml --limit 3
gh run watch <RUN_ID> --repo dekaruntime/<name> --exit-status
```

## Landing a version bump

1. **Pick a version strictly greater than the live latest:**

   ```bash
   curl -sS https://deka.gg/api/registry/<name>.json
   ```

   Setting a version that is already on the registry makes the publish
   step no-op, leaving `deka add` on the old latest forever.

2. **Bump `deka.json` on the PR branch, along with the code.** The PR
   runs the install + typecheck gate; do not merge red.

3. **Merge to `main`.** The publish run does the rest. `deka add <name>`
   resolves the new version as soon as the website deploy that heard the
   dispatch finishes — nobody edits the website repo.

If the package depends on other `@deka/*` modules, those dependency
versions in `deka.json` must already be published, or the pre-merge gate
fails at install time. Publish leaves before dependents:

- `@deka/crypto` before `@deka/jwt`
- `@deka/tcp` and `@deka/tls` before `@deka/http`

## What does **not** publish a package

- Merging a PR that does not change `version` in `deka.json` (no-op)
- Pushing directly to `main`, even with a bumped `version` — the run
  fails and says why
- Pushing a hand-cut tag — tags no longer trigger anything; the workflow
  cuts the tag from the merge commit
- Tagging a version already on the registry (no-op)
- A runtime (`dekaruntime/deka`) release

`bytes` is the reference package — copy its shape (`deka.json`,
`index.ds`, `tests/`, `.gitignore`, and the caller
`.github/workflows/release.yml`).

## Verifying a repo that has never released

`workflow_dispatch` exists so the release path can be proven before
anything real is tagged:

```bash
gh workflow run release.yml --repo dekaruntime/<name> -f wiring_check_version=0.0.0-wiring-check
gh run list --repo dekaruntime/<name> --limit 1
```

The run uploads `0.0.0-wiring-check`, verifies it by ranged GET, then
deletes the object. Deletion matters: the website index discovers
versions by **listing the bucket prefix**, so a throwaway object left in
the bucket would appear on the registry. This exercises the whole path:
org runner assignment, the Cloudflare secrets, the R2 write, and the
ranged-GET verification.

Worth doing because runner visibility is per-repo. A repo with **no
workflow runs at all** has never proven it can see the org self-hosted
runners, and that failure only shows up when you are trying to cut a real
release.

## Website index and curation

`data/registry.ts` in dekaruntime/website carries **curation only**:
which package names exist and their descriptions. It must never carry
version numbers — versions are discovered by listing the R2 prefix at
deploy time, so the index cannot lag behind the bucket. Do not hand-edit
`/packages` dates or download counts — they are not R2 metadata.

Required org/repo secrets for the automated flow:

- `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ENDPOINT` — scoped to
  the `release` environment (same as the old per-repo workflows).
- `WEBSITE_DISPATCH_TOKEN` — able to post `repository_dispatch` to
  dekaruntime/website. Absent, publishes still work; the index updates on
  the website's 2-hourly cron instead of in minutes.
- dekaruntime/website additionally needs read/list credentials on the
  `deka-stdlib` bucket for `probe:r2` (`R2_ENDPOINT`, and an access key
  with `s3:ListBucket`).

## Host catalog vs package tarball

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
  `deka.lock`. The table is delivered by `deka add` / `deka install` into
  `deka.grants.json` (deka#797); `PoolConfig.host_grants` and the
  `DEKA_HOST_GRANTS` environment variable remain as explicit overrides for
  tests and embedded hosts.
- The runtime catalog is generated from
  `permissions::host_bridge` (`js_catalog_json()`) — the single source of
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

## Testsuite

Hats fixtures declare `"packages": ["crypto"]` (etc.) and `deka add`
from the index during dump. They stay red until the index serves the
`.ds` tarball. Do not vendor `index.ds` into a fixture to turn a cell
green.

Dump uses the **published** CLI unless `DEKA_NATIVE` is set. Pair a
local CLI with `DEKA_WASM` from the same commit (`PUBLISH.md`).
