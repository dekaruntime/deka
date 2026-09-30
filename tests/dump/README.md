# Historical browser harness

The aggregate Hats/conformance dump and external corpus release pipeline are
retired. The old collector remains here for historical reference; it is not run
by CI or release publication and is not the current testing contract.

The `scripts/*-e2e.mjs` browser regressions for legacy HMR/Fast Refresh remain in
use until the JavaScript backend is retired. Install their locked dependencies
with `bun install --frozen-lockfile` here. Current native-runtime validation is
documented in the repository's `TESTING.md`.

## Historical collector notes

Corpus `.code` sidecars follow dsc's Hats runner: a file whose trimmed body is
a decimal integer is a process exit code, not formatted source (deka#929).
The collector copies fixture-local `.ds`/`.dsx`/`.css`/`.mjs` into the native
tmpdir the way that runner does, so summon fixtures keep their sibling
`foreign.mjs` (deka#930). Wasm still cannot verify summon; those rows stay
listed host gaps.

The native package cache stores installed modules, `deka.lock`, and the optional
`deka.grants.json` together under `.cache/deka-packages/with-grants-v1/`.
Older cache entries are bypassed because they omitted installer-issued grants.
A grant-free install may legitimately have no grants file. The browser host
forwards `envGranted` to web-ide-kit only when the fixture's `deka.json` lists
a non-empty `security.allow.env` (deka#378 / deka#904); otherwise `process` is
absent, matching native.

`expected-failures.txt` ratchets all shared-host divergences: full diagnostic
lists, formatter output, and per-host expectation results. Listed cases remain
visibly divergent in the dump; unlisted divergences and stale entries fail the
ratchet. web-ide-kit 0.3.4 also strips plain `export function`, so
`modules-export-async-fn` now agrees and has been removed from the ratchet.
The remaining deka#904 row is `modules-import-non-relative-001` (documented
per-host string difference; see `docs/dekascript/missing-module-diagnostics.mdx`).
Native auto-installs `io` when the source imports it; wasm project-compiles the
same fixtures against `stdlib-stubs/io.ds` (`echo(message: string)`) so both
hosts record the same diagnostic SET. Dump comparison uses that shared parser
(order-insensitive); the synthesized wasm `error` slot is logging only.
