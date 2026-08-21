# wasm_hello_rust

Minimal Rust guest module using the `deka-wasm-guest` helper crate.

Build:
```
cargo build --release --target wasm32-unknown-unknown --manifest-path examples/wasm_hello_rust/Cargo.toml
```

Copy the wasm into the php_modules fixture:
```
cp examples/wasm_hello_rust/target/wasm32-unknown-unknown/release/deka_wasm_hello.wasm \
  php_modules/@user/hello/module.wasm
```
