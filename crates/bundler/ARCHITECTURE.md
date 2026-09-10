# Bundler Architecture

## Overview

Deka's bundler is a **JavaScript/TypeScript bundler built on SWC**, featuring module-level caching and optional external module support.

## What We Are (Be Honest!)

**Deka is an SWC-based bundler** - we use SWC for parsing, transformation, and code generation, just like most modern Rust bundlers:

| Bundler | Parser/Transformer | Language |
|---------|-------------------|----------|
| **Rolldown** | oxc | Rust |
| **Rspack** | SWC | Rust |
| **Parcel 2** | SWC | Rust |
| **Turbopack** | SWC | Rust |
| **Deka** | SWC | Rust |

### What SWC Does For Us

- **Parsing**: JavaScript/TypeScript/JSX → AST (`swc_ecma_parser`)
- **Transformation**: TypeScript stripping, JSX → React (`swc_ecma_transforms_*`)
- **Code Generation**: AST → JavaScript code (`swc_ecma_codegen`)
- **Source Maps**: Source mapping support (`swc_common::SourceMap`)
- **Minification**: Code optimization (TODO: `swc_ecma_minifier`)

### What We Built (The Bundler Logic)

- ✅ **Module discovery** - recursive dependency crawling with deduplication
- ✅ **Dependency resolution** - path resolution, node_modules lookup, externals
- ✅ **Dependency graph** - topological sorting with Kahn's algorithm
- ✅ **Module-level caching** - persistent disk cache with mtime validation
- ✅ **Concatenation logic** - combining modules in dependency order
- ✅ **External modules** - skip bundling node_modules for server-side builds
- ✅ **Fast-path optimization** - skip SWC for plain JS in node_modules

## Performance Characteristics

Our performance comes from:
1. **Efficient module resolution** - canonicalized paths, deduplication
2. **Smart caching** - 328x speedup on unchanged builds
3. **External modules** - 3x faster for server-side builds

### Current Benchmark (M4 MacBook, 16GB RAM)

**10K component benchmark (20,002 modules, no minification/source maps):**
- **Rolldown:** 747ms ← fastest (uses oxc parser)
- **Deka:** 899ms ← **20% slower** (uses SWC parser)
- **esbuild:** 1,325ms ← we're 1.5x faster
- **rspack:** 3,075ms ← we're 3.4x faster
- **rollup:** 26,228ms ← we're 29x faster

**Why we're slower than Rolldown:**
- oxc parser is allegedly 3-4x faster than SWC
- They may have more optimized module resolution
- Better lock-free concurrency primitives

**Why we're faster than esbuild/rspack:**
- Module-level caching
- Less overhead in bundling orchestration

## Architecture Diagram

```
Entry point
    │
    ▼
SWC bundler ──► loader reads source and resolver finds dependencies
    │
    ├──► parse and transform JavaScript, TypeScript, and JSX
    ├──► resolve the dependency graph
    └──► emit the bundled JavaScript
```

## Module Resolution

### Resolution Priority

1. **Special modules**: react, react-dom/client, deka/* → skip (handled separately)
2. **External check**: If `bundle_node_modules=false` and bare import → external
3. **Relative imports**: `./foo` or `../bar` → resolve from the importing file's directory
4. **Project alias**: `@/foo` → resolve from the project root
5. **DekaScript sources** (deka#241): extensionless `foo` → `foo.ds`, then `foo/index.ds`. Explicit `foo.ds` is that file. Never `.phpx`.
6. **JS/TS fallback** (mixed graphs only): `.ts`, `.tsx`, `.jsx`, `.js`, `.mjs`, and their `index.*` forms
7. **Absolute imports**: `/foo` → resolve from root
8. **node_modules**: search up the directory tree for `node_modules/pkg`

### External Modules

Controlled by `--target` flag:
- `--target browser` (default): Bundle node_modules (browsers can't resolve bare imports)
- `--target server` or `--target node`: Mark node_modules as external (server-side runtimes can resolve)

## Caching System

### Two-Tier Cache

**1. Memory Cache** (fast path):
```rust
HashMap<PathBuf, CachedModule>
```

**2. Disk Cache** (persistent):
```
~/.config/deka/bundler/cache/
  ├── {hash}.json  # Per-module cache files
  └── graph.json   # Dependency graph
```

### Cache Key & Validation

- **Key**: SHA256 hash of absolute file path
- **Validation**: File mtime (modification time)
- **Invalidation**: Automatic on file change

### What's Cached

```rust
struct CachedModule {
    path: PathBuf,               // Absolute path
    source: String,              // Original source code
    mtime: SystemTime,           // File modification time
    content_hash: String,        // SHA256 of source
    transformed_code: String,    // Transpiled JavaScript
    dependencies: Vec<String>,   // Import specifiers
    resolved_dependencies: Vec<PathBuf>,  // Resolved paths
}
```

### Cache Performance

- **Cold build**: ~900ms (no cache)
- **Warm build**: ~900ms (OS filesystem cache)
- **Cache hit**: **10ms** (328x speedup!)

## SWC Integration Details

### Parsing

```rust
let source_map = Lrc::new(SourceMap::default());
let syntax = if path.ends_with(".tsx") || path.ends_with(".ts") {
    Syntax::Typescript(TsSyntax { tsx: true, ..Default::default() })
} else {
    Syntax::Es(EsSyntax { jsx: true, ..Default::default() })
};

let fm = source_map.new_source_file(FileName::Real(path), source);
let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
let mut parser = Parser::new_from(lexer);
let module = parser.parse_module()?;
```

### Transformation

```rust
let globals = Globals::new();
GLOBALS.set(&globals, || {
    let unresolved_mark = Mark::new();
    let top_level_mark = Mark::new();

    // 1. TypeScript stripping (if .ts/.tsx)
    if is_typescript {
        module.visit_mut_with(&mut strip(top_level_mark));
    }

    // 2. JSX transformation (if .jsx/.tsx)
    if has_jsx {
        let jsx_options = react::Options {
            runtime: Some(react::Runtime::Automatic),
            import_source: Some("react".into()),
            ..Default::default()
        };
        module.visit_mut_with(&mut react(source_map.clone(), None, jsx_options));
    }

    // 3. Resolver (scope analysis)
    module.visit_mut_with(&mut resolver(unresolved_mark, top_level_mark, false));
});
```

### Code Generation

```rust
let mut buf = vec![];
{
    let mut writer = JsWriter::new(source_map.clone(), "\n", &mut buf, None);
    let mut emitter = Emitter {
        cfg: Default::default(),
        cm: source_map.clone(),
        comments: None,
        wr: &mut writer,
    };
    emitter.emit_module(&module)?;
}
let code = String::from_utf8(buf)?;
```

## Performance Optimization Opportunities

### Already Implemented ✅

- [x] Module-level caching
- [x] External modules support
- [x] Fast-path for plain JS

### TODO / Future Work

- [ ] **Source maps**: Output .map files (SWC already creates them internally)
- [ ] **Minification**: Use `swc_ecma_minifier` crate
- [ ] **Incremental builds**: Only rebuild changed modules + dependents
- [ ] **Lock-free data structures**: Replace remaining locks with atomics
- [ ] **Tree shaking**: Dead code elimination
- [ ] **Code splitting**: Multiple output chunks
- [ ] **CSS bundling**: Currently separate from JS bundling

### Parser Alternatives

If we want to close the 20% gap with Rolldown, we could:
- **Option A**: Switch from SWC to oxc parser (3-4x faster parsing)
  - Pros: Faster parsing, newer codebase
  - Cons: Less mature, breaking changes possible, rewrite transforms
- **Option B**: Optimize our bundler logic (module resolution, caching)
  - Pros: Keep SWC ecosystem, incremental improvement
  - Cons: Harder to get massive speedups

## Usage Examples

### Basic Build (Browser)
```bash
deka build ./src/index.jsx
# Output: dist/index-{hash}.js (node_modules bundled)
```

### Server-Side Build (External node_modules)
```bash
deka build ./src/server.ts --target server
# Output: dist/server-{hash}.js (node_modules marked external)
```

### Clear Cache
```bash
deka build --clear-cache
```

### Environment Variables
```bash
DEKA_BUNDLER_CACHE=0          # Disable cache
DEKA_EXTERNAL_NODE_MODULES=1  # Mark node_modules as external (legacy)
```

## Contributing

When contributing to the bundler:

1. **Understand we're SWC-based** - we orchestrate bundling, SWC does parsing/transforms
2. **Test on large codebases** - 10K+ modules reveal performance issues
3. **Profile before optimizing** - use `cargo flamegraph` to find bottlenecks
4. **Consider cache invalidation** - every change must properly invalidate cache
5. **Keep error handling explicit** - malformed modules must fail the bundle

## References

- [SWC Documentation](https://swc.rs/)
- [swc_bundler crate](https://docs.rs/swc_bundler/)
- [Rolldown (oxc-based)](https://rolldown.rs/)
- [esbuild's three.js benchmark](https://github.com/evanw/esbuild/blob/main/Makefile)
