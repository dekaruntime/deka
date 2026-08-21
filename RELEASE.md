# Deka runtime/cli releases

Releases are fully automated through GitHub Actions and published to Cloudflare R2.

## Versioning policy

We follow [Semantic Versioning 2.0](https://semver.org/) with one deliberate constraint: **v1.0.0 is a milestone we will choose explicitly**, not something we drift into. Until then we stay in the `0.x` line and keep incrementing normally:

- **PATCH** (`0.9.0` → `0.9.1`): bug fixes, performance improvements, build/CI fixes, and other backward-compatible corrections.
- **MINOR** (`0.9.x` → `0.10.0`, `0.10.x` → `0.11.0`, etc.): new language features, new stdlib modules, new CLI commands, or breaking changes that are not yet the v1.0 milestone.
- **MAJOR** (`0.x.y` → `1.0.0`): reserved for the stable 1.0 release. We will not bump to v1.0 until we consciously decide the runtime is ready for that milestone.

Every pull request that changes runtime behavior, the CLI, or the browser compiler must bump the version before it merges. Do not wait for a "release branch" or batch multiple changes into a single version bump. Version bumps are part of the change that needs them.

The version lives in each `crates/*/Cargo.toml`. Keep them in lockstep; the release workflow expects a single version for the entire runtime/CLI distribution.

## Triggering a release

Push a git tag matching `v*` after the version has been bumped on `main`. For example:

```sh
git tag -a v0.9.1 -m "deka runtime/cli v0.9.1"
git push origin v0.9.1
```

The `.github/workflows/release.yml` workflow will:

1. Build the `deka` CLI binary in release mode for:
   - `linux-x64`
   - `darwin-x64`
   - `darwin-arm64`
2. Build the browser compiler WASM artifacts (`deka_compiler.wasm`, `deka_diagnostics.wasm`).
3. Compute SHA-256 checksums and write `manifest.json`.
4. Upload CLI binaries and WASM to the `deka-releases` bucket under `v<VERSION>/`.
5. Copy the manifest to `deka-releases/latest.json` so `deka.gg` can point users at the current release.
6. Also copy the WASM files to the `deka-wasm` bucket under `v<VERSION>/` and to `deka-wasm/latest/`, giving the website a stable URL for the pinned browser compiler artifact.

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

`latest.json` (and `v<VERSION>/manifest.json`) has this shape:

```json
{
  "version": "0.9.0",
  "tag": "v0.9.0",
  "commit": "<full-sha>",
  "published_at": "2026-08-16T...Z",
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

`deka.gg` can read this manifest to render download links or to pin the browser compiler artifact used by the tour.

## Notes

- The workflow renames the built binary from `cli` to `deka` when staging artifacts.
- Each platform build uses its own sccache bucket to avoid cross-platform cache poisoning.
- We intentionally do not publish to npm or GitHub Packages; R2 is the single source of truth.
