# Versioning

deka, [dsc](https://github.com/dekaruntime/dsc), [testsuite](https://github.com/dekaruntime/testsuite),
and [tour](https://github.com/dekaruntime/tour) share **one version number**
and bump together, in lockstep. This is RFD 59, implemented in deka#835.

The number does not track any single repo's rate of change. It names a
**known-good set** across all four repos — "0.47.3" means "these four repos,
at these four commits, are a release that was built and tagged together,"
not "the compiler changed 47 times."

## Current line

`0.47.x`.

The corpus tag format moved from `corpus-vX.Y.Z` to the shared `vX.Y.Z`. The
transition accepts both forms; new tags use `vX.Y.Z`.

## Tag everything, build only what changed

Every release tags all four repos, regardless of whether a given repo
changed. Only repos that produce build outputs — deka and dsc — actually
build and publish artifacts. testsuite and tour are content repos with
nothing to compile; they get a tag so the version number stays meaningful
as a cross-repo pointer, but no build runs against them.

A tour typo fix bumps four tags and builds nothing.

## Build before tagging

A tag is only cut after the set builds successfully. There is no "tag now,
build later" path. If a release build fails, the failure blocks the tag —
you get a visibly missing release, never a tag that points at an artifact
that doesn't exist or doesn't match. A failed build never leaves a silently
partial release.

## The lockstep boundary

Lockstep covers the **toolchain** — deka, dsc, testsuite, tour. It does not
extend to anything published independently:

- **Library crates** published to crates.io (e.g. `deka-cli-core`) version by
  their own semver, on their own schedule.
- **Stdlib packages** distributed through the registry version by their own
  semver, on their own schedule.

Consumers of those crates and packages express compatibility through normal
Cargo/registry version ranges, the same as any other dependency. This split
is deliberate: dragging every downstream consumer's version along with the
toolchain was the mistake lockstep is designed to avoid repeating. Lockstep
buys a known-good four-repo set; it is not a license to versionsync the
whole ecosystem.

## Resolving content repos at runtime

```sh
deka self fetch tour
deka self fetch testsuite
```

Each resolves and downloads the content repo at the version matching the
currently running CLI — the same lockstep number, not `latest` on either
repo's default branch.

## Bumping

The version lives in `[workspace.package]` in the root `Cargo.toml`; every
crate uses `version.workspace = true` so they cannot drift internally.

```sh
./scripts/bump-version.sh patch    # or minor, or an explicit X.Y.Z
```

That updates `[workspace.package]`, any crate that still inlines a version,
and `Cargo.lock`. It refuses to go backwards or reuse a version that already
has a `v*` tag. It does not commit or tag.

Open a PR with the bump, merge it with a merge commit, then release per
[PUBLISH.md](PUBLISH.md) — which tags all four repos, builds deka and dsc,
and only lands the tags once that build succeeds.
