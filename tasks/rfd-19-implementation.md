# RFD 19 Implementation Plan

## Goal

Move DekaScript from nominal traits to structural interfaces, add Go-style receiver methods, struct embedding, optional fields, and spread syntax.

## Decisions

- Remove `trait` and `impl`.
- Remove `function` keyword; `fn` is the only function keyword.
- `interface` becomes a structural type with fields and methods.
- Interface fields are read-only by default; `mut` grants write access.
- Methods attach to structs via receiver syntax: `fn (p Person) greet() {}`.
- Mutable receivers: `fn (p mut Person) setName() {}`.
- Struct embedding: `struct Employee { Person }` promotes fields and methods.
- Optional fields: `subtitle?: string`.
- Spread: `{...props}`.

## Phases

### Phase 1: Parser

- Accept `fn` for all function declarations.
- Reject `function` for declarations.
- Parse receiver methods.
- Parse embedded struct fields.
- Parse optional fields.
- Parse spread in objects/JSX props.
- Add/update parser tests.

### Phase 2: AST

- Add receiver method node.
- Add embedded field node.
- Add optional field flag.
- Add spread element node.

### Phase 3: Typechecker

- Structural interface assignability.
- Method resolution with receiver binding.
- Field/method promotion for embedded structs.
- Mutability checking for mutable receivers.

### Phase 4: JS Emitter

- Emit `deka.Struct(id)` factories.
- Attach methods to prototype.
- Wrap mutable methods with `deka.guardMut`.
- Emit embedded struct fields.
- Emit optional fields and spread.

### Phase 5: Remove traits/impl

- Delete parser/typechecker/emitter paths for traits and `impl`.
- Delete or migrate tests.

### Phase 6: Tour and docs

- Update tour examples to new syntax.
- Update docs/dekascript pages.

## Status

- [x] RFD 19 updated
- [x] RFD 22 updated
- [ ] Phase 1: Parser
- [ ] Phase 2: AST
- [ ] Phase 3: Typechecker
- [ ] Phase 4: Emitter
- [ ] Phase 5: Remove traits/impl
- [ ] Phase 6: Tour and docs
