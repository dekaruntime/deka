# Publishing the Deka runtime

This document describes how a new runtime release is produced, where the
artifacts land, and how downstream sites pick them up automatically.

## Overview

A release is driven entirely by GitHub Actions. Pushing an annotated tag matching
`v*` to `dekaruntime/deka` triggers `.github/workflows/release.yml`, which:

1. Runs the Rust test suite on linux-x64, darwin-x64, and darwin-arm64.
2. Builds the browser compiler + diagnostics WASM artifacts.
3. Builds native CLI binaries for linux-x64, darwin-x64, and darwin-arm64.
4. Publishes versioned artifacts to Cloudflare R2.
5. Promotes the `latest` pointers on R2.
6. Dispatches downstream workflows so the tour and conformance sites rebuild.

All jobs run on self-hosted runners and use a shared sccache backend on R2.

## Cutting a release

1. Bump every crate version in `crates/*/Cargo.toml` and update
   `Cargo.lock`.
2. Open a PR with the bump, merge it to `main`.
3. Create and push an annotated tag from `main`:
   ```bash
   git checkout main
   git pull origin main
   git tag -a v0.23.5 -m "release v0.23.5"
   git push origin v0.23.5
   ```
4. Watch the run:
   ```bash
   gh run list --repo dekaruntime/deka --workflow=release.yml
   gh run watch <RUN_ID> --repo dekaruntime/deka --exit-status
   ```

## Release artifacts

After a successful run the following are available on R2:

| Artifact | URL pattern | Consumers |
|---|---|---|
| Release manifest | `https://releases.deka.gg/latest.json` | CLI installers, testsuite native drift checks |
| Versioned release | `https://releases.deka.gg/<VERSION>/...` | Native CLI binaries, WASM files |
| Compiler manifest | `https://wasm.deka.gg/latest/deka-compiler-artifact.json` | Website, testsuite |
| Diagnostics manifest | `https://wasm.deka.gg/latest/deka-diagnostics-artifact.json` | Website, testsuite |
| Compiler WASM | `https://wasm.deka.gg/latest/deka_compiler.wasm` | Website tour, testsuite |
| Diagnostics WASM | `https://wasm.deka.gg/latest/deka_diagnostics.wasm` | Website tour |

The `latest/*` paths are updated atomically at the end of the `publish` job.

## Downstream deployments

The release workflow does not deploy websites directly. It dispatches workflows
in other repositories that consume the artifacts above.

### Website (`dekaruntime/website`)

Triggered by: `workflow_dispatch` from the runtime release job.

Workflow: `.github/workflows/sync-deka-compiler.yml`

What it does:
- Downloads the latest WASM artifacts and metadata from R2.
- Commits them to the website repo (`public/tour/...`).
- Dispatches `.github/workflows/deploy.yml` to rebuild and deploy `deka.gg`.

Because the commit is pushed with `GITHUB_TOKEN`, it does not itself trigger a
push-based deploy, so the sync workflow explicitly queues one.

### Test suite (`dekaruntime/testsuite`)

Triggered by: `workflow_dispatch` from the runtime release job.

Workflow: `.github/workflows/deploy.yml`

What it does:
- Fetches the latest compiler manifest from `wasm.deka.gg/latest`.
- Downloads the matching native CLI for the runner platform from
  `releases.deka.gg/<VERSION>`.
- Runs every conformance test against both the WASM compiler and the native CLI.
- Builds a static Next.js export and deploys it to Cloudflare Workers via
  Wrangler.

The published site (`testsuite.deka.gg`) includes the native-vs-WASM drift
status for every test.

## Required secrets

Secrets live in `dekaruntime/deka` unless noted otherwise.

| Secret | Used by | Required permissions |
|---|---|---|
| `R2_ACCESS_KEY_ID` | `release.yml` | Read/write on the R2 buckets below |
| `R2_SECRET_ACCESS_KEY` | `release.yml` | Read/write on the R2 buckets below |
| `DISPATCH_WEBSITE_SYNC_TOKEN` | `release.yml` | `actions:write` on `dekaruntime/website` |
| `DISPATCH_TESTSUITE_DEPLOY_TOKEN` | `release.yml` | `actions:write` on `dekaruntime/testsuite` |

The two dispatch tokens can be the same fine-grained PAT scoped to both
repositories. They only need to create workflow dispatch events; they do not
need `contents:write`.

Downstream repos also need their own `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID` secrets, but those are unrelated to the runtime release.

## Manual fallback

If a downstream dispatch ever fails, the sites can be redeployed manually:

```bash
# Website
gh workflow run sync-deka-compiler.yml --repo dekaruntime/website --ref main

# Test suite
gh workflow run "Deploy deka test suite" --repo dekaruntime/testsuite --ref main
```

The website sync also runs on an hourly schedule as a backup.

## Verification

After a release, confirm the `latest` artifacts point at the new version:

```bash
curl -s https://wasm.deka.gg/latest/deka-compiler-artifact.json | jq '.compiler.version'
curl -s https://deka.gg/tour/deka-compiler-artifact.json | jq '.compiler.version'
```

Both should match the tag you just pushed.

## Troubleshooting

### Downstream dispatch returns 403

The dispatch token is under-scoped or does not have access to the target repo.
Regenerate it with `actions:write` on both `dekaruntime/website` and
`dekaruntime/testsuite`.

### Website sync commits WASM but does not deploy

The sync workflow dispatches the deploy job with `curl` and `GITHUB_TOKEN`. If
it previously failed with exit code 127, the runner was missing the `gh` CLI.
The workflow now uses `curl` directly, but the `actions:write` permission must
still be granted to `GITHUB_TOKEN`.

### Testsuite shows `nativeAvailable: false`

The native CLI download failed or the binary would not execute on the runner.
Check the `Build site` step logs in the testsuite deploy run for the download
URL and any glibc compatibility warnings.
