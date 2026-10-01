# Native Deka release delivery

Deka 0.60.1 ships the local DekaScript compiler, Rust VM and native window host.
The public executable is built from `crates/deka_cli` as `deka`. The legacy
`crates/cli` executable is retained for historical consumers and is not published
by this pipeline. There is no standalone DSC download or V8 dependency in the
native release.

## Producer

A merged version bump lets `tag-canary.yml` create `vX.Y.Z-canary-<sha>` and invoke
`release.yml`. Do not create stable tags manually. The release workflow:

1. Builds the native CLI for Linux x64, macOS x64 and macOS arm64 using job-level
   sccache and the size-optimized `native` Cargo profile.
2. Exercises each actual executable: source imports, passing/failing app tests,
   compiled executables relocated after source deletion, native UI click handlers.
3. Builds the matched WASM/loader/font package and runs its real compile/click path.
4. Runs local frontend/VM/renderer and relocated Tauri-bundle tests.
5. Publishes checksummed artifacts into `releases.deka.gg/<full-version>/` and
   updates only `canary.json`. npm receives the existing canary dispatch.

`scripts/native-release-manifest.py` creates `release.json` from those exact bytes.
The contract includes `runtime: "deka_vm"`, `cli_abi: 1`, platform filenames,
SHA256s and sizes, and `native_ui` manifest SHA256/size/ABI. The browser manifest
must record the same source commit and base version. There is no `dsc_version`,
legacy compiler-WASM sidecar, or old host declaration artifact in a native release.

## Stable promotion

Sami runs Promote with the validated canary tag. It checks tag/source identity,
native CLI/browser contracts and green validation evidence for that exact commit,
then copies the same bytes to the stable version, updates `latest.json`, and
creates the stable tag. Neither CLI nor WASM is rebuilt on promotion. The binary
reports the source base version, including when distributed as a canary.

## Consumers and landing order

`dekaruntime/create-deka-app` owns npm delivery. Its fixed `publish-runtime.yml`
workflow downloads and verifies the native binaries, preflights every package,
then publishes platform packages, launcher and scaffolder at the same version.
The Deka launcher exposes only `deka`; it neither depends on nor bundles DSC.
Historical DSC packages remain available to their existing consumers. Scheduled
native delivery follows the Deka stable/canary pointers only.

The website follows `latest.json`, downloads and verifies the complete versioned
native browser package, runs tour/browser tests, commits all matched artifacts,
and explicitly dispatches deployment. The installer reads the same release
contract and installs just the native executable, using flags for configuration.

Land the website and npm consumer PRs before promoting the first native stable
release. Until R2 contains a native release, the new installer rejects the old
runtime explicitly. Coordinate website deployment with stable promotion so the
public install link does not temporarily point at an incompatible release.

The old headless compiler consumer needs its own migration; stable native
promotion no longer dispatches that obsolete compiler sync. Retained historical
CLI/compiler/LSP consumers still prevent archival of the entire DSC repository.

## Credentials and responsibilities

Existing R2 release credentials stay in the Deka release environment. Existing
`CASCADE_DISPATCH_TOKEN` dispatches npm; `DISPATCH_WEBSITE_SYNC_TOKEN` dispatches
the website, which also has an hourly fallback. npm uses trusted OIDC publishing
against the existing workflow identity. No credentials move between repositories.
Ava reviews/merges PRs; Sami promotes. Agents do not publish or deploy.

## Verification after promotion

Read `https://releases.deka.gg/latest.json`: version/channel/commit, native runtime
and browser ABI must agree with the release. Read back and SHA256-check the actual
versioned downloads. `npm view @dekaruntime/deka version dependencies --json` must
show the new version without DSC; `npm view create-deka-app version` must match.

Scratch-install through both `curl -fsSL https://deka.gg/install.sh | sh` and
`npx create-deka-app@latest myapp`. Use `npm test`, `npm run build`, `npm run dev`
inside the npm-created app. The single compiled executable must work after
removing its source directory. Verify the live tour loads the matching WASM.
