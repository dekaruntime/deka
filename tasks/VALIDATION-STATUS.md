# PHPX Validation Status

Current state of PHPX validation and what needs to be built.

## Status Update (2026-02-06)

This document originally described an early gap analysis. The current runtime now includes substantial validation coverage:
- Parse errors are integrated with `deka-validation` formatting.
- Semantic validators are active for imports/exports, module rules, and core PHPX restrictions.
- Type system checks are active (assignments, calls, returns, generics/inference paths).
- Struct and enum/match validation is active in the checker/inference pipeline.

Remaining gaps are mostly DX/editor integration and deeper LSP surfacing, tracked in `tasks/UNIFIED-TASKS.md`.

## What We Have ✅

### 1. PHP Parser with PHPX Mode

**Location**: `crates/php-rs/src/parser/`

The php-rs parser already supports PHPX:
```rust
// crates/modules_php/src/modules/php/mod.rs:424
let mut parser = Parser::new_with_mode(lexer, &arena, ParserMode::Phpx);
let program = parser.parse_program();
```

**PHPX parser mode exists** and can parse:
- Type annotations
- Import/export statements
- Struct definitions
- JSX (probably - need to verify)
- Frontmatter templates

### 2. Parse Errors

**Location**: `crates/php-rs/src/parser/ast/mod.rs:14`

```rust
pub struct ParseError {
    pub span: Span,           // Location in source
    pub message: &'static str, // Error message
}

impl ParseError {
    pub fn to_human_readable(&self, source: &[u8]) -> String {
        // Basic Rust-style formatting
    }
}
```

**Current error format** (basic, not using deka-validation):
```
error: Expected ';' after statement
 --> handler.phpx:5:10
  |
5 | $x = 1
  |       ^
```

### 3. Parse Error Collection

Parse errors are collected in the AST:
```rust
let program = parser.parse_program();
if !program.errors.is_empty() {
    // Errors exist but currently just returned as generic string
}
```

### 4. Type Extraction for Bridge

`op_php_parse_phpx_types` already:
- Parses PHPX source
- Extracts type information (structs, functions, type aliases)
- Builds type registry for WIT bridge
- Returns structured type info

**BUT**: It doesn't do semantic validation, just parsing.

---

## What We DON'T Have ❌

### 1. Beautiful Error Formatting

**Current**: Parse errors use basic Rust-style formatting
**Need**: Integration with `deka-validation` for consistent beautiful errors

**Example current error**:
```
error: Expected ';' after statement
 --> handler.phpx:5:10
```

**What we want** (using deka-validation):
```
Validation Error
❌ Syntax Error

┌─ handler.phpx:5:10
│
  5 │ $x = 1
    │       ^ Expected ';' after statement
│
= help: Add semicolon: $x = 1;
│
└─
```

### 2. Semantic Validation

**No validation beyond parsing**. We need:

- ❌ Type checking (type mismatches, inference)
- ❌ PHPX rules (no null, no exceptions, no OOP, no namespace)
- ❌ Import/export validation (module resolution, circular imports)
- ❌ Struct validation (required fields, __construct ban)
- ❌ JSX validation (tag matching, component names)
- ❌ WASM import validation (stub checking)
- ❌ Pattern match exhaustiveness

### 3. Compiler API

**No clean API to trigger validation**. Currently:
```rust
// modules_php/src/modules/php/mod.rs:414
#[op2]
#[serde]
fn op_php_parse_phpx_types(source: String, file_path: String)
    -> Result<BridgeModuleTypes, CoreError> {
    // Just parses and extracts types
    // Returns generic CoreError on failure
    // No structured validation results
}
```

**What we need**:
```rust
// New compiler API
pub fn compile_phpx(source: &str, file_path: &str) -> ValidationResult {
    pub struct ValidationResult {
        pub errors: Vec<ValidationError>,
        pub warnings: Vec<ValidationWarning>,
        pub ast: Option<Ast>,
    }
}
```

### 4. Error Types for PHPX

No PHPX-specific error kinds. We need:
```rust
pub enum ErrorKind {
    SyntaxError,
    TypeError,
    ImportError,
    ExportError,
    NullNotAllowed,     // PHPX rule
    ExceptionNotAllowed, // PHPX rule
    OopNotAllowed,      // PHPX rule
    StructError,
    JsxError,
    WasmImportError,
    NonExhaustiveMatch,
    // ... etc
}
```

### 5. Help Text / Suggestions

Parse errors have messages but no actionable help text:
```rust
pub struct ParseError {
    pub span: Span,
    pub message: &'static str, // Just error message
    // ❌ No help_text field
    // ❌ No error_kind enum
    // ❌ No suggestion field
}
```

---

## What We Need to Build

### Phase 1: Wire Up Existing Parse Errors (Quick Win)

**Goal**: Make parse errors beautiful using deka-validation

**Tasks**:
1. [x] Update `ParseError::to_human_readable()` to use `deka-validation`
2. [x] Add `error_kind` and `help_text` to ParseError
3. [x] Map parser errors to PHPX validation errors

**Files to modify**:
- `crates/php-rs/src/parser/ast/mod.rs` (ParseError struct)
- Add `deka-validation` dependency to `php-rs/Cargo.toml`

**Before**:
```rust
pub struct ParseError {
    pub span: Span,
    pub message: &'static str,
}
```

**After**:
```rust
pub struct ParseError {
    pub span: Span,
    pub message: &'static str,
    pub error_kind: &'static str,  // "Syntax Error", "Type Error", etc.
    pub help_text: &'static str,   // Actionable suggestion
}

impl ParseError {
    pub fn to_validation_error(&self, source: &[u8], file_path: &str) -> String {
        let line_info = self.span.line_info(source)?;
        deka_validation::format_validation_error(
            source,
            file_path,
            self.error_kind,
            line_info.line,
            line_info.column,
            self.message,
            self.help_text,
            self.span.len(),
        )
    }
}
```

**Estimated time**: 1-2 days

---

### Phase 2: Create Validation Infrastructure

**Goal**: Build semantic validation layer

**Tasks**:
1. Create `crates/modules_php/src/validation/` directory
2. Define `ValidationError`, `ValidationResult` types
3. Create validator modules:
   - `syntax.rs` (use existing parse errors)
   - `imports.rs`
   - `exports.rs`
   - `phpx_rules.rs` (no null, no exceptions, etc.)
   - `types.rs` (type checker)
   - `structs.rs`
   - `jsx.rs`
   - `modules.rs`
   - `patterns.rs`

**Files to create**:
- `crates/modules_php/src/validation/mod.rs`
- `crates/modules_php/src/validation/types.rs` (error types)
- `crates/modules_php/src/validation/{syntax,imports,exports,phpx_rules,etc}.rs`

**Estimated time**: 6-8 weeks (see PHPX-VALIDATION.md for breakdown)

---

### Phase 3: Create Compiler API

**Goal**: Expose clean API for CLI and LSP

**Tasks**:
1. Create `crates/modules_php/src/compiler_api.rs`
2. Implement `compile_phpx()` function
3. Run all validation passes
4. Return structured results

**New file**:
```rust
// crates/modules_php/src/compiler_api.rs

pub fn compile_phpx(source: &str, file_path: &str) -> ValidationResult {
    // 1. Parse
    let program = parse_with_mode(source, ParserMode::Phpx);

    // 2. Collect parse errors (use deka-validation)
    let mut errors = program.errors.iter()
        .map(|e| e.to_validation_error(source, file_path))
        .collect();

    // 3. Run semantic validators
    errors.extend(validate_imports(&program.ast));
    errors.extend(validate_phpx_rules(&program.ast));
    errors.extend(check_types(&program.ast));
    // ... etc

    ValidationResult { errors, warnings, ast: program.ast }
}
```

**Estimated time**: 1 week (after Phase 2)

---

## Summary: Current vs. Needed

| Feature | Status | Location | Action Needed |
|---------|--------|----------|---------------|
| **PHPX Parser** | ✅ Exists | `php-rs/src/parser/` | None |
| **Parse Errors** | ✅ Pretty | `php-rs/src/parser/ast/mod.rs` | Add error kind + help text |
| **Error Formatting** | ✅ Partial | N/A | Add parser error mapping + suggestions |
| **Semantic Validation** | ❌ Missing | N/A | Build validators (Phase 2) |
| **Type Checking** | ❌ Missing | N/A | Build type checker |
| **PHPX Rules** | ❌ Missing | N/A | Build PHPX validators |
| **Compiler API** | ✅ Basic | `modules_php/src/compiler_api.rs` | Wire in validation passes |
| **Help Text** | ❌ Missing | N/A | Add to validators |
| **Error Kinds** | ✅ Defined | `modules_php/src/validation/mod.rs` | Wire into validators |

---

## Immediate Next Steps

### Option A: Quick Win (1-2 days)
Start with Phase 1 - make parse errors beautiful:

1. Add `deka-validation` to `php-rs/Cargo.toml`
2. Update `ParseError::to_human_readable()` to use `format_validation_error()`
3. Add `error_kind` and `help_text` fields to ParseError
4. Test with invalid PHPX files

**Result**: Parse errors look beautiful immediately

### Option B: Build Foundation (6-8 weeks)
Jump straight to Phase 2 - build full validation:

1. Create validation infrastructure (types, modules)
2. Implement validators one by one (see PHPX-VALIDATION.md)
3. Wire up deka-validation throughout
4. Build compiler API

**Result**: Complete PHPX validation system

### Recommended: Hybrid Approach

**Week 1**: Phase 1 (quick win with parse errors)
**Weeks 2-8**: Phase 2 (build semantic validation)
**Week 9**: Phase 3 (compiler API)

This gives you beautiful parse errors immediately while building the full system.

---

## Testing Current Parse Errors

To see what parse errors currently look like:

```bash
# Create invalid PHPX file
echo '<?php
$x = 1  // Missing semicolon
function foo() {
    return
}' > test.phpx

# Try to run (will show parse error)
deka run test.phpx
```

Currently this shows basic error. After Phase 1, it will be beautiful.

---

**Status**: Parse infrastructure exists, validation layer needs to be built
**Next**: Start with Phase 1 (beautiful parse errors) then Phase 2 (full validation)
**Timeline**: 1-2 days for Phase 1, 6-8 weeks for complete validation system
