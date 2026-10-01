# Native versioning

`[workspace.package].version` is Deka's source base version. Change it only with
`scripts/bump-version.sh`, in a PR. The native CLI/compiler/VM/browser crates
inherit it. The separate Tauri packaging tool keeps its tool version but its
local dependencies/lockfile follow the workspace.

Canary versions are `X.Y.Z-canary-<commit7>`, with tags prefixed `v`. Stable
promotion copies identical bytes under `X.Y.Z`. CLI banners and browser package
manifests report the source base version; release manifests identify the channel,
full delivery version and exact source commit.

`create-deka-app` and `@dekaruntime/deka*` are stamped to the release delivery
version during publishing. No versions or downloaded binaries are committed in
that npm repository. The website pins the complete matched native browser package.

Legacy DSC pins apply only to historical CLI/compiler consumers; DSC is not an
input to native distribution. The external corpus and tour release lockstep is
retired. Retain Rust tests and `deka test` for app authors.
