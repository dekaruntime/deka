# PHPX Validation System

This document outlines the validation and error checking system for PHPX. These validations will be used by:
1. The PHPX compiler (compile-time checks)
2. The LSP server (real-time editor feedback)
3. The CLI (helpful error messages)

All validation errors will use the `deka-validation` crate for consistent, beautiful error formatting.

**Current Status**: See `VALIDATION-STATUS.md` for what exists vs. what needs to be built.

## Checklist Sync (2026-02-06)

The core implementation checklists in this file have been synced with current runtime behavior:
- Parser + syntax validations: implemented
- Import/export + module isolation validations: implemented
- PHPX language rule validations (no classes/namespace/throw/new in PHPX contexts): implemented
- Struct/enum/match typechecking paths: implemented

Open work is now concentrated in tooling/editor diagnostics quality and integration layers.

**TL;DR**: We have the parser and parse errors, but need to:
1. Integrate existing parse errors with `deka-validation` (Phase 1 - quick win)
2. Build semantic validation layer (Phase 2-7 in this document)
3. Create compiler API (Phase 8)

---

## Architecture

### Validation Layers

```
┌─────────────────────────────────────────────────┐
│  PHPX Source Code (.phpx file)                  │
└─────────────────┬───────────────────────────────┘
                  │
         ┌────────▼────────┐
         │   Parser        │ ← Syntax validation
         │   (php-rs)      │
         └────────┬────────┘
                  │
         ┌────────▼────────┐
         │   AST           │
         └────────┬────────┘
                  │
    ┌─────────────┴─────────────┐
    │                           │
┌───▼────────┐         ┌────────▼─────┐
│  Semantic  │         │  Type        │
│  Validator │         │  Checker     │
└───┬────────┘         └────────┬─────┘
    │                           │
    └─────────────┬─────────────┘
                  │
         ┌────────▼────────┐
         │  Validation     │
         │  Results        │
         │  (Errors +      │
         │   Warnings)     │
         └────────┬────────┘
                  │
         ┌────────▼────────┐
         │  deka-          │
         │  validation     │ ← Beautiful formatting
         │  formatter      │
         └─────────────────┘
```

### Validation Result Structure

```rust
// crates/modules_php/src/validation/mod.rs

pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
    pub ast: Option<Ast>,
}

pub struct ValidationError {
    pub kind: ErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub help_text: String,
    pub underline_length: usize,
    pub severity: Severity,
}

pub enum ErrorKind {
    // Syntax
    SyntaxError,
    UnexpectedToken,

    // Types
    TypeError,
    TypeMismatch,
    UnknownType,

    // Imports/Exports
    ImportError,
    ExportError,
    ModuleNotFound,
    CircularImport,

    // PHPX Rules
    NullNotAllowed,
    ExceptionNotAllowed,
    OopNotAllowed,
    NamespaceNotAllowed,

    // Structs
    StructError,
    InvalidStructLiteral,

    // JSX
    JsxError,
    InvalidJsxExpression,

    // WASM
    WasmImportError,
    MissingWitStubs,

    // Pattern Matching
    NonExhaustiveMatch,
}

pub enum Severity {
    Error,
    Warning,
    Info,
}
```

---

## Phase 1: Core Syntax Validation

### Task 1.1: Parse Error Recovery
**Goal**: Catch syntax errors and provide helpful messages

**What to validate**:
- [x] Unclosed braces, brackets, parentheses
- [x] Invalid tokens
- [x] Unexpected end of file
- [x] Malformed expressions

**Example errors**:
```phpx
// Missing closing brace
function foo() {
    echo 'hello';
//  ^ SyntaxError: Expected '}', found end of file
//    help: Add closing brace for function body
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/syntax.rs`
- [x] Implement `validate_syntax(source: &str, ast: &Program) -> Vec<ValidationError>`
- [x] Use php-rs parser error recovery
- [x] Map parser errors to validation errors
- [x] Add helpful suggestions for common mistakes

**Files to create**:
- `crates/modules_php/src/validation/mod.rs`
- `crates/modules_php/src/validation/syntax.rs`
- `crates/modules_php/src/validation/types.rs` (error types)

---

### Task 1.2: Import Statement Validation
**Goal**: Validate import syntax and semantics

**What to validate**:
- [x] Import at top of file (before other code)
- [x] Valid import syntax
- [x] Named imports: `import { foo, bar } from 'module'`
- [x] WASM imports: `import { fn } from '@user/mod' as wasm`
- [x] Module path format (no relative paths with `../`)
- [x] Unused imports (warning)
- [x] Duplicate imports

**Example errors**:
```phpx
$x = 1;
import { foo } from 'bar';
// ^ ImportError: import must appear before other code
//   help: Move import statements to the top of the file

import { greet } from '../utils';
//                     ^ ImportError: Relative imports not supported
//   help: Use module names from php_modules/ instead: 'utils'

import { unused } from 'module';
// ^ Warning: Unused import 'unused'
//   help: Remove unused import or use it in your code

import { foo } from 'mod';
import { bar } from 'mod';
// ^ ImportError: Duplicate import from 'mod'
//   help: Combine imports: import { foo, bar } from 'mod'
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/imports.rs`
- [x] Implement `validate_imports(source: &str, file_path: &str) -> (Vec<ValidationError>, Vec<ValidationWarning>)`
- [x] Check import placement (AST position)
- [x] Validate module paths
- [x] Track used imports (mark on usage)
- [x] Detect duplicates

---

### Task 1.3: Export Statement Validation
**Goal**: Validate export syntax and semantics

**What to validate**:
- [x] Export only functions, constants, types
- [x] No duplicate exports
- [x] Exported names actually exist
- [x] Re-export syntax validation
- [x] Template files: no explicit exports (auto-exported as Component)

**Example errors**:
```phpx
export function foo() {}
export function foo() {}
// ^ ExportError: Duplicate export 'foo'
//   help: Remove duplicate export

export function bar() {}
// No function 'bar' defined
// ^ ExportError: Export 'bar' not defined
//   help: Define function before exporting

// In frontmatter template file
---
export function Component() {}
// ^ ExportError: Explicit exports not allowed in template files
//   help: Template component is auto-exported. Remove 'export' keyword.
---
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/exports.rs`
- [x] Implement `validate_exports(source: &str, file_path: &str, ast: &Program) -> Vec<ValidationError>`
- [x] Track exported names
- [x] Check for duplicates
- [x] Verify definitions exist
- [x] Special handling for template files

---

## Phase 2: Type System Validation

### Task 2.1: Type Annotation Syntax Validation
**Goal**: Validate type annotation syntax

**What to validate**:
- [x] Valid type names
- [x] Generic syntax: `Option<T>`, `Result<T, E>`, `array<T>`
- [x] Object shape syntax: `Object<{ field: Type }>`
- [x] Type alias syntax: `type Name = ...`
- [x] Union types (limited): `int|float`
- [x] No nullable types (`?T`, `T|null` are banned)

**Example errors**:
```phpx
function foo(?string $name): void {}
//           ^ TypeError: Nullable types not allowed in PHPX
//   help: Use Option<string> instead: Option<string> $name

type User = { name: string, age?: int };
$user: User = { name: 'Sam' };
// Valid - optional fields allowed in object types

type MaybeUser = User|null;
//                    ^ TypeError: null type not allowed in PHPX
//   help: Use Option<User> instead
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/type_syntax.rs`
- [x] Implement `validate_type_annotations(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Reject `null`, `?T`, `T|null` syntax
- [x] Validate generic parameter syntax
- [x] Check object shape syntax

---

### Task 2.2: Type Checking
**Goal**: Validate types match across assignments, function calls, returns

**What to validate**:
- [x] Variable assignment type matches
- [x] Function parameter types match arguments
- [x] Return type matches returned value
- [x] Binary operation types compatible
- [x] Struct field types match literal values
- [x] Safe widening only (int → float allowed, not reverse)

**Example errors**:
```phpx
int $x = "hello";
//       ^ TypeError: Type mismatch
//   Expected: int
//   Got: string
//   help: Assign an integer value or change type to string

function greet(string $name): void {
    return $name;
//  ^ TypeError: Return type mismatch
//    Expected: void
//    Got: string
//    help: Remove return value or change return type to string

function add(int $a, int $b): int {
    return $a + $b;
}

$result = add("1", "2");
//            ^ TypeError: Argument type mismatch
//   Expected: int
//   Got: string
//   help: Pass integer arguments: add(1, 2)
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/type_checker.rs`
- [x] Implement type inference engine (via php-rs PHPX typeck)
- [x] Implement `check_types(program: &Program, source: &str, file_path: Option<&str>) -> Vec<ValidationError>`
- [x] Build type environment (symbol table)
- [x] Infer types for expressions
- [x] Check compatibility at assignments/calls/returns
- [x] Track widening rules

**This is the most complex task - may need 2-3 weeks**

---

### Task 2.3: Generic Type Validation
**Goal**: Validate generic parameters and constraints

**What to validate**:
- [x] Generic parameters are used
- [x] Generic constraints are satisfied
- [x] Type arguments provided where required
- [x] Constraint syntax: `T: Reader`

**Example errors**:
```phpx
function identity<T>(T $x): T {
    return $x;
}

$result = identity("hello");  // OK - T inferred as string

function unused<T>(int $x): int {
//              ^ Warning: Generic parameter T is unused
//   help: Remove unused generic parameter or use it in function signature

interface Reader {
    function read(): string;
}

function process<T: Reader>(T $r): string {
    return $r.read();
}

struct NotReader {}

$nr = NotReader {};
process($nr);
// ^ TypeError: Type NotReader does not satisfy constraint Reader
//   help: NotReader must implement read(): string
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/generics.rs`
- [x] Implement `validate_generics(program: &Program, source: &str) -> (Vec<ValidationError>, Vec<ValidationWarning>)`
- [x] Track generic parameters
- [x] Check constraints
- [x] Infer type arguments

---

## Phase 3: PHPX-Specific Rules

### Task 3.1: No Null Rule
**Goal**: Ban null literals and null checks

**What to validate**:
- [x] No `null` literals
- [x] No `=== null` or `!== null` comparisons
- [x] No `is_null()` calls
- [x] Suggest `Option<T>` instead

**Example errors**:
```phpx
$user = null;
//      ^ NullNotAllowed: null literals not allowed in PHPX
//   help: Use Option::None instead

if ($user === null) {
//          ^ NullNotAllowed: null comparison not allowed
//   help: Use pattern matching: match ($user) { Option::None => ... }

if (is_null($user)) {
//  ^ NullNotAllowed: is_null() not allowed in PHPX
//   help: Use Option::is_none() method or pattern matching
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/phpx_rules.rs`
- [x] Implement `validate_no_null(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Scan AST for null literals
- [x] Scan for null comparisons
- [x] Scan for is_null() calls

---

### Task 3.2: No Exception Rule
**Goal**: Ban throw/try/catch

**What to validate**:
- [x] No `throw` statements
- [x] No `try/catch/finally` blocks
- [x] Suggest `Result<T, E>` instead
- [x] Allow `panic()` for unrecoverable errors

**Example errors**:
```phpx
throw new Exception("error");
// ^ ExceptionNotAllowed: throw not allowed in PHPX
//   help: Return Result::Err($error) instead

try {
// ^ ExceptionNotAllowed: try/catch not allowed in PHPX
//   help: Use Result<T, E> and pattern matching instead
    riskyOperation();
} catch (Exception $e) {
    handleError($e);
}

// Correct:
function riskyOperation(): Result<int, string> {
    if ($failed) {
        return Result::Err("operation failed");
    }
    return Result::Ok(42);
}
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/phpx_rules.rs`
- [x] Implement `validate_no_exceptions(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Scan for throw statements
- [x] Scan for try/catch/finally

---

### Task 3.3: No OOP Rule
**Goal**: Ban classes, traits, extends, implements

**What to validate**:
- [x] No `class` declarations
- [x] No `trait` declarations
- [x] No `extends` keyword
- [x] No `implements` keyword
- [x] No `new` keyword
- [x] No `interface` inheritance (structural interfaces only)
- [x] Suggest structs instead

**Example errors**:
```phpx
class User {
// ^ OopNotAllowed: class declarations not allowed in PHPX
//   help: Use struct instead: struct User { ... }
    public string $name;
}

$user = new User();
//      ^ OopNotAllowed: 'new' keyword not allowed in PHPX
//   help: Use struct literal: User { $name: 'Sam' }

interface Reader extends BaseReader {
//                ^ OopNotAllowed: interface inheritance not allowed
//   help: PHPX uses structural interfaces. Just define the methods.
}
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/phpx_rules.rs`
- [x] Implement `validate_no_oop(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Scan for class/trait/interface declarations
- [x] Scan for extends/implements
- [x] Scan for new keyword

---

### Task 3.4: No Namespace Rule
**Goal**: Ban namespace declarations and top-level use

**What to validate**:
- [x] No `namespace` declarations
- [x] No top-level `use` statements
- [x] Suggest import/export instead

**Example errors**:
```phpx
namespace App\Controllers;
// ^ NamespaceNotAllowed: namespace declarations not allowed in PHPX
//   help: Use import/export module system instead

use App\Models\User;
// ^ NamespaceNotAllowed: top-level use not allowed in PHPX
//   help: Use import statement: import { User } from 'models/user'
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/phpx_rules.rs`
- [x] Implement `validate_no_namespace(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Scan for namespace declarations
- [x] Scan for top-level use statements

---

## Phase 4: Struct Validation

### Task 4.1: Struct Definition Validation
**Goal**: Validate struct definitions

**What to validate**:
- [x] No `__construct` in PHPX structs
- [x] Field defaults are constant expressions
- [x] Field type annotations are valid
- [x] No duplicate field names
- [x] Struct composition (`use A`) is valid

**Example errors**:
```phpx
struct Point {
    $x: int;
    $y: int;

    function __construct(int $x, int $y) {
//  ^ StructError: __construct not allowed in PHPX structs
//    help: Use struct literals instead: Point { $x: 1, $y: 2 }
        $this->x = $x;
    }
}

struct Config {
    $host: string = getDefaultHost();
//                  ^ StructError: Field defaults must be constant expressions
//    help: Use literal value or compute in constructor context
}

struct User {
    $name: string;
    $name: string;
//  ^ StructError: Duplicate field 'name'
//    help: Remove duplicate field definition
}
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/structs.rs`
- [x] Implement `validate_struct_definitions(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Check for __construct
- [x] Validate field defaults are constants
- [x] Check for duplicate fields
- [x] Validate composition

---

### Task 4.2: Struct Literal Validation
**Goal**: Validate struct literal syntax and usage

**What to validate**:
- [x] All required fields provided
- [x] No extra fields
- [x] Field types match values
- [x] Shorthand syntax valid
- [x] Nested struct literals valid

**Example errors**:
```phpx
struct Point {
    $x: int;
    $y: int;
}

$p = Point { $x: 1 };
//           ^ StructError: Missing required field 'y'
//   help: Add missing field: Point { $x: 1, $y: 2 }

$p = Point { $x: 1, $y: 2, $z: 3 };
//                         ^ StructError: Unknown field 'z' for struct Point
//   help: Remove extra field or add to struct definition

$p = Point { $x: "hello", $y: 2 };
//               ^ TypeError: Type mismatch for field 'x'
//   Expected: int
//   Got: string
//   help: Use integer value: Point { $x: 1, $y: 2 }

// Shorthand
$x = 1;
$p = Point { $x, $y: 2 };  // OK

$p = Point { $z };
//           ^ StructError: Variable $z does not match any field
//   help: Shorthand requires variable name to match field name
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/structs.rs`
- [x] Implement `validate_struct_literals(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Check required fields
- [ ] Reject extra fields
- [ ] Validate field types
- [ ] Handle shorthand syntax

---

## Phase 5: JSX Validation

### Task 5.1: JSX Syntax Validation
**Goal**: Validate JSX element syntax

**What to validate**:
- [x] Valid tag names
- [x] Matching opening/closing tags
- [x] Valid attribute syntax
- [x] Fragment syntax: `<>...</>`
- [x] Self-closing tags

**Example errors**:
```phpx
<div>
    <span>Hello</div>
//              ^ JsxError: Closing tag 'div' does not match opening tag 'span'
//   help: Change to </span>

<Component invalid-attr={$x}>
//         ^ JsxError: Invalid attribute name
//   help: Use camelCase: invalidAttr or data-invalid-attr

<div/>    // OK - self-closing
<div>     // ERROR - missing closing tag
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/jsx.rs`
- [x] Implement `validate_jsx_syntax(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Check tag matching
- [x] Validate attribute names
- [x] Check self-closing vs paired tags

---

### Task 5.2: JSX Expression Validation
**Goal**: Validate PHPX expressions in JSX

**What to validate**:
- [x] Expression syntax: `{$var}`, `{$obj.field}`
- [x] Object literals require double braces: `{{ key: 'value' }}`
- [x] No statements in expressions (only expressions; `if/foreach` must be rewritten)
- [x] Disambiguate JSX vs `<`/`>` comparisons in JSX-capable contexts:
  - [x] Require spaces around comparison operators (e.g. `$a < $b`) in JSX-capable expressions.
  - [x] Allow legacy tight comparisons outside JSX expressions (no JSX ambiguity).
  - [x] Emit help text: “Add spaces around `<`/`>` to avoid JSX ambiguity.”

**Example errors**:
```phpx
<div>
    {$user = getUser()}
//  ^ JsxError: Statements not allowed in JSX expressions
//   help: Extract to variable before JSX: $user = getUser(); <div>{$user}</div>

<Component config={ host: 'localhost' }>
//                  ^ JsxError: Object literal requires double braces
//   help: Use {{ host: 'localhost' }}

<div>
    {if $x { <p>yes</p> }}
//     ^ JsxError: Statements not allowed in JSX expressions
//   help: Use: {$x ? <p>yes</p> : null}

<div>
    {foreach $items as $item { <li>{$item}</li> }}
//           ^ JsxError: Statements not allowed in JSX expressions
//   help: Use: {array_map(fn($item) => <li>{$item}</li>, $items)}
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/jsx.rs`
- [x] Implement `validate_jsx_expressions(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Check expression syntax
- [x] Reject statements inside JSX expressions
- [x] Detect statements in expressions
- [x] Check object literal braces

---

### Task 5.3: Component Validation
**Goal**: Validate component usage

**What to validate**:
- [x] Component names are capitalized (or imported)
- [x] Built-in tags are lowercase
- [x] Component props match definition (if available)
- [x] Special components: `<Link>`, `<Hydration>`, `<ContextProvider>`

**Example errors**:
```phpx
function userCard($props) {
//       ^ Warning: Component function should be capitalized
//   help: Rename to UserCard for JSX usage: <UserCard />

<userCard name="Sam" />
// ^ JsxError: Unknown component 'userCard'
//   help: Import component or use capitalized name: <UserCard />

<Link to="/path">
//    ^ JsxError: Missing required prop 'layout' for Link
//   help: Add layout prop: <Link to="/path" layout="main">

<header>Menu</header>  // OK - lowercase = DOM element
<Header>Menu</Header>  // OK - uppercase = component
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/jsx.rs`
- [x] Implement `validate_components(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Check component naming
- [x] Validate props (if type info available)
- [x] Track imported components

---

### Task 5.4: Frontmatter Template Validation
**Goal**: Validate frontmatter templates

**What to validate**:
- [x] Frontmatter starts at beginning of file
- [x] Proper `---` delimiters
- [x] No explicit exports in template files (under `php_modules/`)
- [x] Template section is valid JSX
- [x] Imports in frontmatter only

**Example errors**:
```phpx
// Not at start of file
echo 'hi';
---
// ^ TemplateError: Frontmatter must start at beginning of file
//   help: Move --- to line 1

---
import { Link } from 'component/dom';
$title = 'Home';

<div>Template</div>
// ^ TemplateError: Missing closing ---
//   help: Add --- after frontmatter code

// In php_modules/ui/card.phpx
---
export function foo() {}
// ^ TemplateError: Explicit exports not allowed in template modules
//   help: Template is auto-exported as Component. Remove export.
---
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/jsx.rs`
- [x] Implement `validate_frontmatter(source: &str, file_path: &str) -> Vec<ValidationError>`
- [x] Check frontmatter position
- [x] Validate delimiters
- [x] Check for exports (if in php_modules/)
- [x] Validate imports placement

---

## Phase 6: Module System Validation

### Task 6.1: Module Resolution Validation
**Goal**: Validate module paths resolve correctly

**What to validate**:
- [x] Module exists in `php_modules/`
- [x] Module has valid entry point
- [x] Circular imports detected
- [x] Import/export names match

**Example errors**:
```phpx
import { foo } from 'nonexistent';
//                   ^ ModuleNotFound: Module 'nonexistent' not found
//   help: Available modules: string, array, component/core, component/dom

import { missing } from 'string';
//       ^ ImportError: 'missing' is not exported from 'string'
//   help: Available exports: strlen, substr, trim, ...

// In moduleA.phpx
import { foo } from './moduleB';

// In moduleB.phpx
import { bar } from './moduleA';
// ^ CircularImport: Circular import detected
//   moduleA -> moduleB -> moduleA
//   help: Refactor to remove circular dependency
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/modules.rs`
- [x] Implement `validate_module_resolution(source: &str, file_path: &str) -> Vec<ValidationError>`
- [x] Scan php_modules/ for available modules
- [x] Build dependency graph
- [x] Detect cycles
- [x] Check export names

---

### Task 6.2: WASM Import Validation
**Goal**: Validate WASM imports

**What to validate**:
- [x] `@user/module` format
- [x] `deka.json` exists
- [x] `module.wasm` exists
- [x] `.d.phpx` stub file exists (suggest generating if missing)
- [x] Imported names exist in stubs

**Example errors**:
```phpx
import { greet } from '@user/missing' as wasm;
//                    ^ WasmImportError: WASM module '@user/missing' not found
//   help: Available WASM modules: @user/hello, @user/crypto
//   Or create new module: deka wasm init @user/missing

import { greet } from '@user/hello' as wasm;
// Missing .d.phpx stub
// ^ MissingWitStubs: Type stubs missing for '@user/hello'
//   help: Generate stubs: deka wasm stubs @user/hello

import { invalid } from '@user/hello' as wasm;
//       ^ WasmImportError: 'invalid' is not exported from '@user/hello'
//   help: Available exports: greet, make_user, get_position
```

**Implementation**:
- [x] Add to `crates/modules_php/src/validation/modules.rs`
- [x] Implement `validate_wasm_imports(source: &str, file_path: &str) -> Vec<ValidationError>`
- [x] Scan php_modules/@*/ for WASM modules
- [x] Check deka.json, module.wasm, .d.phpx
- [x] Parse .d.phpx for exported names
- [x] Suggest deka wasm commands

---

## Phase 7: Pattern Matching Validation

### Task 7.1: Match Exhaustiveness Checking
**Goal**: Ensure all enum cases are handled

**What to validate**:
- [x] Enum match covers all cases
- [x] No unreachable match arms
- [x] Variable binding in match arms
- [x] Payload destructuring correct

**Example errors**:
```phpx
enum Status {
    case Pending;
    case Running(int $pid);
    case Complete(int $code);
    case Failed(string $error);
}

function handle(Status $status): void {
    match ($status) {
        Status::Pending => echo 'waiting',
        Status::Running($pid) => echo "running: {$pid}",
        Status::Complete($code) => echo "done: {$code}",
        // Missing Status::Failed
    }
//  ^ NonExhaustiveMatch: Match is not exhaustive
//    Missing cases: Status::Failed
//    help: Add case: Status::Failed($error) => ...

    match ($status) {
        Status::Pending => echo 'waiting',
        Status::Running($x, $y) => echo $x,
//                      ^ TypeError: Wrong number of payload fields
//    Status::Running has 1 field ($pid), but 2 are destructured
//    help: Use: Status::Running($pid) => ...
    }
}
```

**Implementation**:
- [x] Create `crates/modules_php/src/validation/patterns.rs`
- [x] Implement `validate_match_exhaustiveness(program: &Program, source: &str) -> Vec<ValidationError>`
- [x] Build enum case registry
- [x] Check match coverage
- [x] Validate payload destructuring
- [x] Detect unreachable arms

---

## Phase 8: Integration and Testing

### Task 8.1: Integrate Validation into Compiler
**Goal**: Wire validation into PHPX compilation pipeline

**What to implement**:
- [x] Create `crates/modules_php/src/compiler_api.rs`
- [x] Expose `compile_phpx(source: &str, file_path: &str) -> ValidationResult`
- [x] Run all validation passes in order:
  1. Syntax validation
  2. Import/export validation
  3. Type checking
  4. PHPX rules
  5. Struct validation
  6. JSX validation
  7. Module resolution
  8. Pattern matching
- [x] Collect all errors and warnings
- [x] Format with `deka-validation`
- [x] Return structured result

**Files to create**:
- `crates/modules_php/src/compiler_api.rs`

**Implementation**:
```rust
// compiler_api.rs
pub fn compile_phpx(source: &str, file_path: &str) -> ValidationResult {
    let mut errors = vec![];
    let mut warnings = vec![];

    // 1. Parse
    let ast = match parse_phpx(source) {
        Ok(ast) => ast,
        Err(parse_errors) => {
            errors.extend(parse_errors);
            return ValidationResult { errors, warnings, ast: None };
        }
    };

    // 2. Validate syntax
    errors.extend(validate_syntax(source, &ast));

    // 3. Validate imports/exports
    let (import_errors, import_warnings) = validate_imports(source, file_path);
    errors.extend(import_errors);
    warnings.extend(import_warnings);
    errors.extend(validate_exports(source, file_path, &ast));

    // 4. Type checking
    errors.extend(check_types(&ast));

    // 5. PHPX rules
    errors.extend(validate_no_null(&ast, source));
    errors.extend(validate_no_exceptions(&ast, source));
    errors.extend(validate_no_oop(&ast, source));
    errors.extend(validate_no_namespace(&ast, source));

    // 6. Structs
    errors.extend(validate_struct_definitions(&ast));
    errors.extend(validate_struct_literals(&ast));

    // 7. JSX
    errors.extend(validate_jsx_syntax(&ast));
    errors.extend(validate_jsx_expressions(&ast));
    errors.extend(validate_components(&ast));

    // 8. Modules
    errors.extend(validate_module_resolution(&ast, file_path));
    errors.extend(validate_wasm_imports(&ast));

    // 9. Patterns
    errors.extend(validate_match_exhaustiveness(&ast));

    ValidationResult { errors, warnings, ast: Some(ast) }
}
```

---

### Task 8.2: Create Comprehensive Test Suite
**Goal**: Test all validation rules

**What to test**:
- [ ] Create test file for each validation rule
- [ ] Positive tests (valid code passes)
- [ ] Negative tests (invalid code caught)
- [ ] Error message quality
- [ ] Help text accuracy
- [ ] Edge cases

**Test structure**:
```rust
// crates/modules_php/tests/validation_tests.rs

#[test]
fn test_no_null_literal() {
    let source = r#"$x = null;"#;
    let result = compile_phpx(source, "test.phpx");

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ErrorKind::NullNotAllowed);
    assert!(result.errors[0].help_text.contains("Option::None"));
}

#[test]
fn test_struct_literal_missing_field() {
    let source = r#"
    struct Point {
        $x: int;
        $y: int;
    }
    $p = Point { $x: 1 };
    "#;
    let result = compile_phpx(source, "test.phpx");

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ErrorKind::StructError);
    assert!(result.errors[0].message.contains("Missing required field 'y'"));
}
```

**Test files to create**:
- `crates/modules_php/tests/validation/syntax_tests.rs`
- `crates/modules_php/tests/validation/import_tests.rs`
- `crates/modules_php/tests/validation/type_tests.rs`
- `crates/modules_php/tests/validation/phpx_rules_tests.rs`
- `crates/modules_php/tests/validation/struct_tests.rs`
- `crates/modules_php/tests/validation/jsx_tests.rs`
- `crates/modules_php/tests/validation/module_tests.rs`
- `crates/modules_php/tests/validation/pattern_tests.rs`

---

### Task 8.3: Update deka-validation for PHPX
**Goal**: Enhance deka-validation with PHPX-specific formatting

**What to add**:
- [x] Support for multiple errors in single file
- [x] Color coding by error kind
- [x] Code suggestions (auto-fix hints)
- [x] Reference links to PHPX docs

**Example enhanced output**:
```
Validation Errors (3 found)

❌ NullNotAllowed

┌─ handler.phpx:5:10
│
  5 │ $user = null;
    │          ^^^^ null literals not allowed in PHPX
│
= help: Use Option::None instead
= docs: https://deka.dev/docs/phpx/option
│
└─

❌ ImportError

┌─ handler.phpx:1:26
│
  1 │ import { greet } from '@user/missing' as wasm;
    │                          ^^^^^^^^^^^^^^ WASM module '@user/missing' not found
│
= help: Available WASM modules: @user/hello, @user/crypto
        Or create new module: deka wasm init @user/missing
│
└─

❌ TypeError

┌─ handler.phpx:10:15
│
 10 │ $result = add("1", "2");
    │               ^^^ Type mismatch
    │ Expected: int
    │ Got: string
│
= help: Pass integer arguments: add(1, 2)
= suggestion: $result = add(1, 2);
│
└─
```

**Files to update**:
- `/Users/samifouad/Projects/deka/deka-validation/src/lib.rs`

**New features**:
- [x] `format_multiple_errors()` - Format list of errors
- [x] Color codes by severity (error=red, warning=yellow, info=blue)
- [x] `format_with_suggestion()` - Include code fix suggestions
- [x] `format_with_docs_link()` - Add doc links

---

## Implementation Priority

### Recommended Order:

**Week 1-2: Foundation**
1. Task 1.1: Parse error recovery ✅ Foundation
2. Task 1.2: Import validation ✅ Core feature
3. Task 1.3: Export validation ✅ Core feature
4. Task 3.1-3.4: PHPX rules (no null, no exceptions, no OOP, no namespace) ✅ Critical

**Week 3-4: Type System**
5. Task 2.1: Type annotation syntax ✅ Required for types
6. Task 2.2: Type checking ✅ Most complex, most valuable
7. Task 2.3: Generic validation ✅ Depends on type checker

**Week 5: Structs**
8. Task 4.1: Struct definitions ✅ Core feature
9. Task 4.2: Struct literals ✅ Core feature

**Week 6: JSX**
10. Task 5.1: JSX syntax ✅ Component system
11. Task 5.2: JSX expressions ✅ Component system
12. Task 5.3: Component validation ✅ Component system
13. Task 5.4: Frontmatter validation ✅ Template support

**Week 7: Modules**
14. Task 6.1: Module resolution ✅ Module system
15. Task 6.2: WASM imports ✅ Extension system

**Week 8: Advanced**
16. Task 7.1: Match exhaustiveness ✅ Safety
17. Task 8.1: Compiler integration ✅ Wire everything
18. Task 8.2: Test suite ✅ Quality
19. Task 8.3: deka-validation updates ✅ Polish

---

## Success Metrics

We'll know validation is working when:

1. **Syntax Errors**: Caught immediately with helpful messages
2. **Type Errors**: Caught at compile time, not runtime
3. **PHPX Rules**: Enforced (no null, no exceptions, no OOP)
4. **Import Errors**: Missing modules caught with suggestions
5. **WASM Integration**: Stub checking works, suggests deka wasm commands
6. **Pattern Matching**: Non-exhaustive matches caught
7. **Test Coverage**: >90% of validation rules tested
8. **Developer Feedback**: Error messages are helpful and actionable

---

## Next Steps

1. **Review this document** - Confirm validation scope
2. **Start with foundation** - Implement tasks 1.1-1.3, 3.1-3.4
3. **Test with real PHPX** - Use examples/ directory for testing
4. **Iterate on error messages** - Make them beautiful and helpful
5. **Wire into LSP** - Once validation works, integrate into DX-TASKS.md plan

---

**Status**: Planning phase
**Last Updated**: 2026-02-01
