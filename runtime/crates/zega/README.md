# Vendored Zega crates

These crates are vendored from `tana/zega` commit
`703e5066dd2b22a00c1bd2a21ede3e040c0d0feb`.

Deka vendors the embedded database crates so fleet build machines can compile
from a clean clone without credentials for the private Zega repository.

To sync:

1. Export `zega-core`, `zega-graph`, `zega-kv`, `zega-parser`, and `zega-wal`
   from the desired upstream commit.
2. Replace the matching directories here.
3. Convert upstream workspace dependencies in each `Cargo.toml` back to the
   explicit versions and sibling path dependencies used here.
4. Update the commit above and run the Deka build and test gates.
