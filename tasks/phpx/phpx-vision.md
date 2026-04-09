# PHPX Vision

PHPX is a modern language that uses the PHP runtime as its compatibility layer.
It is not “PHP with extras” — it is a new language with its own rules, ergonomics,
and guarantees, while still being able to run existing PHP code when needed.

## Why PHPX exists
- **Instant adoption**: existing PHP apps can run without a build step.
- **Clean slate**: new projects get a safer, more expressive language.
- **Real-time runtime**: no bundling or compile step required.

## Core principles
- **Strict by default** in `.phpx` (predictable types and errors).
- **Value semantics** for new primitives (structs, object literals).
- **Explicit modules** (no implicit globals; unused imports are errors).
- **Compositional design** over inheritance.
- **Interoperable by design** with PHP (opt‑in bridging).

## What makes PHPX different
- **Structs (value semantics)** instead of PHP classes for core data models.
- **Object literals** as a natural JSON‑like value type.
- **Strong typing with inference** to keep code terse and safe.
- **Modern module system** (`import`/`export`) scoped per file.
- **JSX + VNode runtime** as a first‑class UI primitive.
- **Option/Result** as the default error model (exceptions discouraged).

## Compatibility strategy
- `.php` stays PHP‑compatible; PHPX features are opt‑in.
- `.phpx` is the recommended greenfield path.
- Bridging is explicit at module boundaries.

## North‑star outcomes
- PHPX is a **credible alternative** to TypeScript + Node for web apps.
- PHPX can **replace PHP** for new projects without sacrificing runtime speed.
- A developer can **start in PHP, grow into PHPX**, and stay in one ecosystem.

## Non‑goals
- Recreating full PHP OOP inheritance inside `.phpx`.
- Requiring a build step to run projects.

## Short statement
**PHPX is a modern, typed, value‑oriented language that runs instantly and
ships with a PHP compatibility layer — enabling zero‑migration adoption and
greenfield productivity in the same runtime.**
