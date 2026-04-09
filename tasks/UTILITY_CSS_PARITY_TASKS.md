# Utility CSS Tailwind Parity Tasks

Status: Deferred
Owner: Runtime/HTTP
Scope: `crates/http/src/utility_css.rs`

## Goal

Reach practical Tailwind utility parity in Deka runtime utility CSS so apps can drop CDN/imported Tailwind while keeping near-identical output and DX.

## Priority DX Track (Do First)

- [ ] DX.1 Implement in-house utility-class IntelliSense in `phpx_lsp`
- [ ] DX.2 Add class completion for PHPX `class`/`className` attributes
- [ ] DX.3 Add unknown utility diagnostics (warning by default)
- [ ] DX.4 Create a shared utility registry consumed by runtime + LSP
- [ ] DX.5 Add sync guard test so runtime utility map and LSP completions cannot drift

## Non-Goals (for this track)

- No Tailwind plugin execution in-process (unless explicitly added later)
- No JS build step requirement
- No regression to current no-build runtime flow

## Phase 1: Baseline and Coverage

- [ ] 1.1 Define target version/scope (`Tailwind v4 core utility subset`)
- [ ] 1.2 Add class corpus fixture (`tests/utility_css/tailwind_core_classes.txt`)
- [ ] 1.3 Add coverage report tool (`supported`, `unsupported`, `%`)
- [ ] 1.4 Add CI check to publish coverage artifact
- [ ] 1.5 Freeze acceptance threshold for first milestone (example: `80% core coverage`)

## Phase 2: Core Utility Expansion

- [ ] 2.1 Complete spacing/sizing matrix (m/p inset w/h min/max full scales)
- [ ] 2.2 Complete color/token matrix (bg/text/border/ring/placeholder)
- [ ] 2.3 Complete border/radius/ring/shadow scales
- [ ] 2.4 Complete typography utilities (font, leading, tracking, align, decoration)
- [ ] 2.5 Complete layout utilities (position, display, overflow, z-index, isolation)
- [ ] 2.6 Complete flex/grid utilities (grow/shrink/basis/order/auto-flow/placement)

## Phase 3: Variant and Selector Parity

- [ ] 3.1 Expand pseudo variants (`focus-visible`, `disabled`, `visited`, etc.)
- [ ] 3.2 Expand responsive variants (all configured breakpoints)
- [ ] 3.3 Add group/peer/data/aria selector variant support
- [ ] 3.4 Add variant chaining tests (`md:hover:...`, `dark:focus:...`)

## Phase 4: Arbitrary Values and Advanced Rules

- [ ] 4.1 Harden bracket syntax parser (`w-[..]`, `grid-cols-[..]`, `shadow-[..]`)
- [ ] 4.2 Add arbitrary color/value normalization and escaping safety
- [ ] 4.3 Add calc()/var()/url() safe handling
- [ ] 4.4 Add strict invalid-class diagnostics mode (optional)

## Phase 5: Visual Fidelity and Regression Testing

- [ ] 5.1 Add reference render set comparing Tailwind vs Deka output
- [ ] 5.2 Add Playwright screenshot diff suite for representative pages
- [ ] 5.3 Add regression suite for utility-css preflight behavior
- [ ] 5.4 Add stability checks for cache/output determinism

## Phase 6: Performance and Runtime Hardening

- [ ] 6.1 Profile generation cost on large pages
- [ ] 6.2 Optimize class extraction and rule generation hot paths
- [ ] 6.3 Add bounded cache eviction policy for `.cache/utility-css`
- [ ] 6.4 Add production/dev toggles and docs for perf tradeoffs

## Phase 7: DX and Configuration

- [ ] 7.1 Expand `deka.css.json` schema docs
- [ ] 7.2 Add optional strict mode (`unknown utility warnings`)
- [ ] 7.3 Add optional class allow/deny lists for org policies
- [ ] 7.4 Document migration guide: Tailwind CDN/import -> runtime utility CSS

## Milestones

- [ ] M1: Coverage report + 80% core class support
- [ ] M2: Variant parity for common app patterns
- [ ] M3: Visual parity on linkhash and one additional production-like app
- [ ] M4: Default-on parity confidence for MVP docs

## Exit Criteria

- [ ] Coverage meets target threshold
- [ ] Visual diff suite stable
- [ ] No critical runtime regressions in serve/dev/HMR flow
- [ ] Docs published for users migrating off Tailwind import/CDN
