# DekaScript LSP

The DekaScript language server is launched with `deka lsp --stdio`. Its public
language identifier is `dekascript`, and it discovers, resolves, indexes, and
renames `.ds` files only.

## Build and run

```sh
cargo build --release -p dekascript_lsp
cargo run --release -p dekascript_lsp
```

## Editor integration

Configure the editor to start the release `cli` binary with `lsp --stdio`, use
the `dekascript` language id, and associate the server only with `.ds` files.
The server accepts an optional initialization setting:

```json
{
  "dekascript": { "target": "server" }
}
```

`DEKA_MODULE_ROOT` can override the project root used for `php_modules`
resolution. Workspace roots come from `workspaceFolders` or `rootUri`, falling
back to the current directory.
