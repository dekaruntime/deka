# bench toolchain

`deka` here is a **main-build**, not a GitHub release. Islands hydration (#948) is on `main` and has not shipped in a released `deka` yet. `dsc` is the released compiler copied next to it so the CLI can find a sibling.

```bash
cargo build --release -p cli --features dev-server
cp ../../target/release/cli ./deka
cp "$HOME/.deka/bin/dsc" ./dsc
./deka --version --verbose
./dsc --version
```

`deka`, `dsc`, and `version.txt` are gitignored. Record the printed versions in the bench results table.
