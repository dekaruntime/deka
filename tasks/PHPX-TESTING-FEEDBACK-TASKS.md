# PHPX Testing Feedback Tasks

## Purpose
Track quirks and correctness gaps found during real-world PHPX testing, then fix them in shared parser/typechecker/runtime + LSP paths.

## Workflow
- Add a minimal repro first.
- Fix in shared code (`crates/php-rs` and/or `crates/phpx_lsp*`) so runtime + LSP stay in sync.
- Add regression tests.
- Update docs only after behavior is stable.

## Active Items

### 1) Local Typed Variable Bindings
Repro:

```php
$input: string = "world"
```

Current behavior:
- Parses as generic syntax error (`Missing semicolon`) in some contexts.

Tasks:
- [x] Parser: support `$name: Type = expr` and `$name: Type`.
- [x] AST: represent typed local declarations explicitly.
- [x] Typechecker: enforce assignment compatibility and later re-assignment rules.
- [x] Validation: emit targeted diagnostics (not generic semicolon errors).
- [x] LSP diagnostics: surface correct spans/messages.
- [x] LSP completion/hover: include declared local type.
- [x] Tests: parser + typecheck + LSP fixtures (positive and negative).
- [x] Docs: add final syntax/examples in PHPX type docs.

### 2) JSX Boolean Shorthand vs Typed Props
Repro:

```php
<Hello name />
```

Current behavior:
- Treated as boolean `true` and rendered as `1` for string-typed prop use cases.

Tasks:
- [x] Typechecker: reject boolean shorthand when expected prop type is non-bool (for example `string`).
- [x] Error message: explain shorthand semantics and suggest `name="..."` or `name={$expr}`.
- [x] LSP diagnostics: show same error in-editor.
- [x] Tests: runtime/typecheck + LSP regression coverage.

### 3) Named Import DX (Invalid Export + Multiline Imports)
Repro:

```php
import {
  stat
} from 'db'
```

Current behavior:
- Invalid named imports could degrade to generic/secondary editor warnings.
- Multiline import clauses were not consistently handled in completion/diagnostics.

Tasks:
- [x] LSP diagnostics: resolve unknown named exports with focused `Import Error`.
- [x] LSP diagnostics: include `did you mean ...` suggestions for close export names.
- [x] LSP diagnostics: include available export preview in message.
- [x] LSP completion: support named export completion inside multiline import clauses.
- [x] Tests: add regression coverage for multiline import diagnostics/completion.
