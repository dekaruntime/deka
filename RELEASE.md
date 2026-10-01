# Release native Deka

Use `scripts/bump-version.sh patch|minor|major` in a PR. It updates workspace
versions and lockfiles, including the separate build-time packaging tool.
Ava merges reviewed PRs; every main merge cuts a canary automatically unless
that base version has already been promoted. Sami promotes validated canary
bytes through Actions → Promote; do not push stable tags yourself.

The native CLI, browser WASM and npm scaffolder use one source base version.
DSC and the retired external testsuite/tour repositories are no longer release
inputs. Native Rust tests and public `deka test` remain required.

See [PUBLISH.md](PUBLISH.md) for artifact contracts, coordinated consumer landing,
required credentials and fresh-install verification. Windows prebuilt delivery
remains outside the existing supported Linux/macOS release matrix.
