# Deka Bundler

A high-performance JavaScript/TypeScript bundler built on SWC with parallel processing and module-level caching.

## What is This?

**Deka is an SWC-based bundler** - similar to Rspack, Parcel 2, and Turbopack. We use SWC for parsing, transformation, and code generation, while building custom bundler logic on top.

### What SWC Does
- Parse JavaScript/TypeScript/JSX
- Transform TypeScript → JavaScript
- Transform JSX → React
- Generate optimized code
- Create source maps
- Minify code

### What Deka Does
- Parallel module discovery (10 concurrent workers)
- Dependency resolution and graph building
- Module-level caching (328x speedup on warm builds)
- External module support (3x faster for server builds)
- Smart concatenation and code generation

## Performance

**Benchmark: 10K components (20,002 modules) on M4 MacBook**

| Bundler | Time | vs Deka |
|---------|------|---------|
| Rolldown | 747ms | 1.2x faster (uses oxc) |
| **Deka** | **899ms** | **baseline** |
| esbuild | 1,325ms | 1.5x slower |
| rspack | 3,075ms | 3.4x slower |
| rollup | 26,228ms | 29x slower |

## Usage

### Browser Build (default)
```bash
deka build ./src/index.jsx
# Bundles all node_modules for browser deployment
# Output: dist/index-{hash}.js
```

### Server Build (external node_modules)
```bash
deka build ./src/server.ts --target server
# Marks node_modules as external (3x faster)
# Output: dist/server-{hash}.js
```

### With Source Maps (TODO)
```bash
deka build ./src/index.jsx --sourcemap
# Output: dist/index-{hash}.js + dist/index-{hash}.js.map
```

### With Minification (TODO)
```bash
deka build ./src/index.jsx --minify
# Uses SWC's built-in minifier
```

## Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for detailed architecture documentation including:
- SWC integration details
- Parallel processing design
- Caching strategy
- Module resolution algorithm
- Performance optimization opportunities

## Current Features

- ✅ **Parallel bundling** - 10 concurrent workers with tokio
- ✅ **Module caching** - Persistent disk cache + in-memory cache
- ✅ **External modules** - Optional node_modules bundling
- ✅ **Fast-path optimization** - Skip SWC for plain JS files
- ✅ **TypeScript support** - Full TS/TSX transformation
- ✅ **JSX support** - React automatic runtime
- ✅ **Dependency graph** - Topological sorting
- ✅ **Cache invalidation** - mtime-based validation

## Planned Features

- ⏳ **Source maps** - Output .map files (SWC support ready)
- ⏳ **Minification** - SWC minifier integration (crate added)
- ⏳ **Incremental builds** - Rebuild only changed modules
- ⏳ **Tree shaking** - Dead code elimination
- ⏳ **Code splitting** - Multiple output chunks

## Why SWC?

**Every Rust bundler uses a parser/transformer library:**

| Bundler | Parser | Notes |
|---------|--------|-------|
| Rolldown | oxc | 3-4x faster than SWC, less mature |
| Rspack | SWC | Production-ready, Webpack-compatible |
| Parcel 2 | SWC | Used in production at scale |
| Turbopack | SWC | Next.js team's choice |
| Deka | SWC | Battle-tested, full ecosystem |

We chose SWC because:
- ✅ **Mature** - battle-tested in production
- ✅ **Complete** - source maps, minification, all features
- ✅ **Maintained** - active development by Vercel
- ✅ **Ecosystem** - extensive plugin system
- ✅ **Fast enough** - within 20% of oxc

To switch to oxc (like Rolldown) would require:
- Rewriting all transforms (TypeScript, JSX)
- Rebuilding source map integration
- Accepting breaking changes risk
- ~2-4 weeks of work for 20% speedup

**Current assessment:** SWC is the right choice for production use.

## Environment Variables

```bash
DEKA_BUNDLER_CACHE=0          # Disable cache (default: enabled)
DEKA_PARALLEL_BUNDLER=0       # Use standard bundler (default: parallel)
DEKA_EXTERNAL_NODE_MODULES=1  # Mark node_modules external (legacy, use --target)
```

## Contributing

When contributing to the bundler:

1. **Read [ARCHITECTURE.md](./ARCHITECTURE.md)** to understand the design
2. **Test on large codebases** - 10K+ modules reveal issues
3. **Profile before optimizing** - use `cargo flamegraph`
4. **Consider cache invalidation** - ensure changes invalidate correctly
5. **Think about parallelism** - avoid shared mutable state

## Benchmarks

### 10K Component Benchmark
- 20,002 total modules (10K components + 10K icons)
- Pure bundling (no minification or source maps yet)
- 899ms on M4 MacBook (16GB RAM)
- Competitive with Rolldown (20% gap)

### Cache Performance
- **Cold build**: 899ms (no cache)
- **Warm build**: 899ms (OS filesystem cache)
- **Cache hit**: 10ms (328x speedup!)

### External Modules
- **Browser** (default): 899ms, 20,002 modules
- **Server** (`--target server`): 303ms, 10,002 modules (3x faster)

## License

Part of the Deka project.
