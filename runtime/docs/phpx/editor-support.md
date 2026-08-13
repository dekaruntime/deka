---
title: "PHPX Editor Support"
section: "phpx"
category: "reference"
categoryLabel: "Reference"
categoryOrder: 100
---

# PHPX Editor Support

## Overview
PHPX editor support is split into two layers:
- Syntax layer: grammar/highlighting for `.phpx` files.
- Semantic layer: `phpx_lsp` for diagnostics, hover, completion, navigation, references, and rename.

## Install

### Zed
1. Install the local dev extension from `extensions/phpx` (Install Dev Extension).
2. Build binaries:
```sh
cargo build --release -p cli -p phpx_lsp
```
3. Add in `~/.config/zed/settings.json`:
```json
{
  "lsp": {
    "phpx-lsp": {
      "binary": {
        "path": "/absolute/path/to/deka/target/release/phpx_lsp"
      }
    }
  }
}
```

### VS Code
1. Install extension from `extensions/vscode-phpx`:
```sh
cd extensions/vscode-phpx
npm install
npm run package
code --install-extension vscode-phpx-0.2.0.vsix
```
2. Build binaries:
```sh
cargo build --release -p cli -p phpx_lsp
```
3. Optional settings (`settings.json`):
```json
{
  "phpx.lsp.path": "/absolute/path/to/deka/target/release/phpx_lsp",
  "phpx.lsp.args": []
}
```
If `phpx.lsp.path` is empty, the extension runs `deka lsp`.

### Neovim
With `nvim-lspconfig`:
```lua
require('lspconfig').phpx_lsp.setup({
  cmd = { '/absolute/path/to/deka/target/release/phpx_lsp' },
  filetypes = { 'phpx' },
  root_dir = require('lspconfig.util').root_pattern('deka.lock', 'php_modules', '.git'),
})
```

Reusable Neovim integration files live in `extensions/nvim-phpx`:
- `extensions/nvim-phpx/lsp.lua`
- `extensions/nvim-phpx/snippets/phpx.lua`
- `extensions/nvim-phpx/ftdetect/phpx.lua`
- `extensions/nvim-phpx/README.md`

### Helix
Configure language server in `languages.toml`:
```toml
[language-server.phpx-lsp]
command = "/absolute/path/to/deka/target/release/phpx_lsp"

[[language]]
name = "phpx"
scope = "source.phpx"
file-types = ["phpx"]
language-servers = ["phpx-lsp"]
```

## Feature Matrix
| Feature | Zed | VS Code | Neovim | Helix |
|---|---|---|---|---|
| Syntax highlighting | Yes | Yes | Via grammar plugin | Via grammar |
| Diagnostics | Yes | Yes | Yes | Yes |
| Hover | Yes | Yes | Yes | Yes |
| Completion | Yes | Yes | Yes | Yes |
| Go to definition | Yes | Yes | Yes | Yes |
| References | Yes | Yes | Yes | Yes |
| Rename | Yes | Yes | Yes | Yes |

## Troubleshooting
- No diagnostics:
  - Rebuild `phpx_lsp`: `cargo build --release -p phpx_lsp`
  - Ensure editor points to the rebuilt binary.
  - Restart editor.
- Diagnostics are noisy/colored:
  - Update to latest `phpx_lsp` release build. LSP diagnostics are now plain text.
- Unknown language for `.phpx`:
  - Verify extension is installed and active.
  - Confirm file association includes `.phpx`.
- Imports unresolved in LSP:
  - Ensure workspace has `php_modules/`.
  - Ensure either local `php_modules/` exists or `PHPX_MODULE_ROOT` points at a root containing `php_modules/`.
- Named export completion missing in `import { ... } from 'mod'`:
  - Rebuild and restart language server.
  - Confirm imported module has explicit `export` declarations.
  - Confirm module resolves to `.phpx`/`index.phpx` under `php_modules/`.
- CLI/runtime mismatch:
  - Rebuild CLI: `cargo build --release -p cli`
  - Use `target/release/cli` (wired local `deka`).

## Dev Mode Workflow (VS Code + Zed)
1. Rebuild binaries:
```sh
cargo build --release -p cli -p phpx_lsp
```
2. Start runtime in dev mode:
```sh
deka serve --dev main.phpx
```
3. Verify dev runtime:
  - HTTP serves normally.
  - HMR WS endpoint is `/_deka/hmr`.
  - In dev mode, HTML responses get a dev client script injected automatically.
4. Editor loop:
  - Keep `phpx_lsp` pointed at `target/release/phpx_lsp`.
  - On parser/type/LSP changes: rebuild and restart language server.

## Zed Notes
- For local grammar development, install the PHPX extension as a dev extension from `extensions/phpx`.
- If grammar build fails, ensure `extension.toml` grammar source points to a reachable local path/revision for dev use.

## Known Limitations
- Some editor-specific visual behaviors (fold markers, indent behavior) still require manual verification.
- VS Code extension packaging currently includes unbundled `node_modules`.
- Cross-platform release packaging for `phpx_lsp` is not automated yet.

## Neovim Textobjects
With the PHPX tree-sitter grammar/queries loaded, the following are available:
- function outer/inner
- struct outer/inner
- JSX outer/inner
- parameter outer
