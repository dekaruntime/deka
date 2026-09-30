# Versioning during the native runtime consolidation

The next milestone is **0.60.0**. Deka's workspace version is the source of truth;
use `scripts/bump-version.sh` to change it. The frontend, native UI model,
renderer and VM now live in this workspace and are built from one source commit.

The previous four-repository lockstep between Deka, DSC, testsuite and tour is
retired. The standalone corpus is no longer part of a release. Do not manufacture
new DSC/corpus versions merely to match Deka's version number.

The legacy CLI still pins an external DSC in `scripts/dsc-version`. That pin
remains an explicit compatibility dependency until its remaining commands are
migrated; consolidation of the native frontend alone does not remove it. The
legacy tour checkout likewise retains an immutable content pin until the website
showcase migration is complete. See `tasks/consolidation-1197.md`.

Canary creation and human-controlled stable promotion remain separate. Agents do
not tag, publish or promote. The tour's new runtime delivery pipeline is a
separate follow-up: compiler, VM, renderer, glue and examples must be verified as
a compatible artifact set before the website selects it.
