# Deka runtime/cli releases

Releases are fully automated through GitHub Actions and published to Cloudflare R2.

## Versioning policy

We follow [Semantic Versioning 2.0](https://semver.org/) with one deliberate constraint: **v1.0.0 is a milestone we will choose explicitly**, not something we drift into. Until then we stay in the `0.x` line and keep incrementing normally:

- **PATCH** (`0.9.0` → `0.9.1`): bug fixes, performance improvements, build/CI fixes, and other backward-compatible corrections.
- **MINOR** (`0.9.x` → `0.10.0`, `0.10.x` → `0.11.0`, etc.): new language features, new stdlib modules, new CLI commands, or breaking changes that are not yet the v1.0 milestone.
- **MAJOR** (`0.x.y` → `1.0.0`): reserved for the stable 1.0 release. We will not bump to v1.0 until we consciously decide the runtime is ready for that milestone.

Every pull request that changes runtime behavior, the CLI, or the browser compiler must bump the version before it merges. Do not wait for a "release branch" or batch multiple changes into a single version bump. Version bumps are part of the change that needs them.

The version lives in `[workspace.package]` in the root `Cargo.toml`. Every crate uses `version.workspace = true` so they cannot drift. Do not hand-edit thirty crate files.

## Bumping

Run this **before** opening the bump PR (and before tagging):

```sh
./scripts/bump-version.sh patch    # or minor, or an explicit 0.30.0
```

That updates `[workspace.package]`, any crate that still inlines a version, and `Cargo.lock`. It refuses to go backwards, to reuse a version that already has a `v*` tag, or to land at or below the latest published tag — the failure mode behind `v0.26.1` (tagged on a 0.26.0 tree) and `v0.28.0` (tagged on a 0.27.0 tree).

It does not commit or tag. Open a PR with the bump, merge it, then tag from `main`.

## Triggering a release

See [rfd#68](https://github.com/dekaruntime/rfd/issues/68). Every build is a
**canary**; a **stable** release only ever republishes an already-built
canary's bytes. There is no more "push a `vX.Y.Z` tag by hand" step —
`.github/workflows/release.yml` now fails fast on a plain `vX.Y.Z` ref with
an error pointing here.

1. Merge your bump PR (see Bumping, above) to `main`. Nothing else to do:
   `.github/workflows/tag-canary.yml` tags the merge commit
   `vX.Y.Z-canary-<7-hex-sha>` and starts `.github/workflows/release.yml` for
   that tag.
2. Watch the canary build:
   ```sh
   gh run list --repo dekaruntime/deka --workflow=release.yml
   gh run watch <RUN_ID> --repo dekaruntime/deka --exit-status
   ```
   `release.yml` builds the three CLI binaries + browser WASM, smoke-tests
   the *built* binaries (no compilation, ~30s), publishes them to R2 under
   the canary's own path plus `canary.json` / `deka-wasm`'s `canary/*`
   pointers, and dispatches `create-deka-app` (npm `canary` dist-tag) and
   `testsuite-site`. It never touches `latest.json` / `latest/*`.
   Afterwards, a conformance dump runs against the built artifacts and
   writes `validation.json` — this does not block the canary publish, but it
   is what `promote.yml` checks before promoting.
3. Test the canary (it is a real, installable build). Iterate by merging
   fixes to `main`; each merge yields a new canary of the same base version
   automatically.
4. Promote: **Actions → Promote → Run workflow**, with the canary tag as
   input (e.g. `v0.59.0-canary-d5661ed`).

### Promotion, step by step

`.github/workflows/promote.yml` enforces these gates, in order, all
fail-closed with a clear `::error::`:

1. The input matches `vX.Y.Z-canary-<7-hex-sha>`, that tag exists, and no
   `vX.Y.Z` tag exists yet.
2. `<canary-version>/release.json` exists on `releases.deka.gg` and its
   `commit` equals the canary tag's commit.
3. `<canary-version>/validation.json` exists, names the same commit, and its
   `gate` is `"green"` — a green result from a different canary of the same
   version cannot be reused.
4. The canary's `dsc_version` has no prerelease suffix (a stable deka cannot
   pin a canary dsc).

If every gate passes: `promote.yml` copies the canary's bytes to the stable
path in both R2 buckets (no rebuild — same bytes, only the manifest's
`version`/`tag`/`channel`/`promoted_from` change), writes `latest.json` and
the WASM `latest/*` pointers, creates and pushes the `vX.Y.Z` tag at the
canary's own commit, and notifies all five downstreams (website, npm
`latest`, testsuite-site, draftwriter, headless).

The old workflow (build the three CLI binaries, browser WASM, checksum,
`manifest.json`/`release.json`, upload) still happens — just once per
version line, inside the canary's `release.yml` run, not on every promotion:

1. Build the `deka` CLI binary in release mode for:
   - `linux-x64`
   - `darwin-x64`
   - `darwin-arm64`
2. Build the browser compiler WASM artifacts (`deka_compiler.wasm`, `deka_diagnostics.wasm`).
3. Compute SHA-256 checksums and write `release.json`.
4. Upload CLI binaries and WASM to the `deka-releases` bucket under
   `<VERSION>/` (no `v` prefix — the R2 releases bucket never had one) and to
   the `deka-wasm` bucket under `v<VERSION>/` (the wasm bucket keeps its
   existing `v` prefix).
5. Publish `canary.json` / `canary/*` (canary) — `promote.yml` is the only
   thing that later writes `latest.json` / `latest/*` (stable).

## Required GitHub secrets

Set these in the `dekaruntime/deka` repository settings under **Settings → Secrets and variables → Actions**:

| Secret | Description |
|--------|-------------|
| `CF_ACCOUNT_ID` | Cloudflare account ID. Used to build `https://<account-id>.r2.cloudflarestorage.com`. |
| `R2_ACCESS_KEY_ID` | R2 S3-compatible access key ID. Needs read/write on the sccache buckets and the releases bucket. |
| `R2_SECRET_ACCESS_KEY` | R2 S3-compatible secret access key. |

## Required R2 buckets

Create one bucket per platform for sccache and buckets for releases and WASM artifacts:

- `deka-sccache-linux-x64`
- `deka-sccache-darwin-x64`
- `deka-sccache-darwin-arm64`
- `deka-releases`
- `deka-wasm`

The sccache buckets can be empty initially; sccache will populate them on first build.

## R2 token permissions

The token used by GitHub Actions needs these permissions:

- **Object Storage: Read/Write** on all five buckets.

If you want a narrower token, scope it to the buckets above.

## Manifest format

`latest.json` / `canary.json` (and `<VERSION>/release.json`, the file this
repo calls its manifest — there is no separately-named `manifest.json`) has
this shape. rfd#68 added `channel`, `base_version`, `dsc_version`,
`corpus_sha` and `promoted_from`; every other key is unchanged:

```json
{
  "version": "0.59.0-canary-d5661ed",
  "tag": "v0.59.0-canary-d5661ed",
  "commit": "<full-sha>",
  "published_at": "2026-08-16T...Z",
  "channel": "canary",
  "base_version": "0.59.0",
  "dsc_version": "0.53.5",
  "corpus_sha": "59e9c9ea6ea5c10779116ac30bef0c482fc94c98cc8d8ddb295ad2805500d30a",
  "promoted_from": null,
  "binaries": {
    "linux-x64": { "name": "deka-linux-x64", "sha256": "..." },
    "darwin-x64": { "name": "deka-darwin-x64", "sha256": "..." },
    "darwin-arm64": { "name": "deka-darwin-arm64", "sha256": "..." }
  },
  "wasm": {
    "compiler": "deka_compiler.wasm",
    "compiler_sha256": "...",
    "diagnostics": "deka_diagnostics.wasm",
    "diagnostics_sha256": "..."
  }
}
```

A promoted (`stable`) manifest is the identical document with `version` /
`tag` rewritten to the plain `vX.Y.Z`, `channel: "stable"`, and
`promoted_from` set to the canary tag it came from — `commit`, `dsc_version`,
`corpus_sha` and every checksum are untouched, because it is the same build.

`promote.yml` also reads (never writes, except its own success) a sibling
`validation.json` per canary version, written by `release.yml`'s
`dump-conformance` job:

```json
{
  "tag": "v0.59.0-canary-d5661ed",
  "commit": "<full-sha>",
  "base_version": "0.59.0",
  "dsc_version": "0.53.5",
  "corpus_sha": "...",
  "gate": "green",
  "summary": {
    "fail": 0,
    "unexpectedDivergences": 0,
    "staleDivergences": 0,
    "unexpectedFailures": 0,
    "staleFailures": 0
  },
  "run_url": "https://github.com/dekaruntime/deka/actions/runs/...",
  "written_at": "2026-08-16T...Z"
}
```

`deka.gg` can read the release manifest to render download links or to pin the browser compiler artifact used by the tour.

## Notes

- The workflow renames the built binary from `cli` to `deka` when staging artifacts.
- Each platform build uses its own sccache bucket to avoid cross-platform cache poisoning.
- R2 is the single source of truth. The npm packages (`create-deka-app`, `@dekaruntime/deka*`) are copies of these R2 binaries, published by `dekaruntime/create-deka-app` after each release at the same version number; nothing is compiled there.
