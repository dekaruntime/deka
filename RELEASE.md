# Deka runtime/cli releases

Releases are fully automated through GitHub Actions and published to Cloudflare R2.

## Triggering a release

Push a git tag matching `v*`. For example:

```sh
git tag -a v0.9.0 -m "deka runtime/cli v0.9.0"
git push origin v0.9.0
```

The `.github/workflows/release.yml` workflow will:

1. Build the `deka` CLI binary in release mode for:
   - `linux-x64`
   - `darwin-x64`
   - `darwin-arm64`
2. Build the browser compiler WASM artifacts (`deka_compiler.wasm`, `deka_diagnostics.wasm`).
3. Compute SHA-256 checksums and write `manifest.json`.
4. Upload everything to the R2 bucket under `runtime/v<VERSION>/`.
5. Copy the manifest to `runtime/latest.json` so `deka.gg` can point users at the current release.

## Required GitHub secrets

Set these in the `dekaruntime/deka` repository settings under **Settings → Secrets and variables → Actions**:

| Secret | Description |
|--------|-------------|
| `CF_ACCOUNT_ID` | Cloudflare account ID. Used to build `https://<account-id>.r2.cloudflarestorage.com`. |
| `R2_ACCESS_KEY_ID` | R2 S3-compatible access key ID. Needs read/write on the sccache buckets and the releases bucket. |
| `R2_SECRET_ACCESS_KEY` | R2 S3-compatible secret access key. |

## Required R2 buckets

Create one bucket per platform for sccache and one bucket for releases:

- `deka-sccache-linux-x64`
- `deka-sccache-darwin-x64`
- `deka-sccache-darwin-arm64`
- `deka-releases`

The sccache buckets can be empty initially; sccache will populate them on first build.

## R2 token permissions

The token used by GitHub Actions needs these permissions:

- **Object Storage: Read/Write** on all four buckets.

If you want a narrower token, scope it to the buckets above.

## Manifest format

`runtime/latest.json` (and `runtime/v<VERSION>/manifest.json`) has this shape:

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
