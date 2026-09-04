# Parallel Bundler Roadmap

## Current State
- **Build time**: 1.4-2.8s (10K components)
- **Simple apps**: 2-4ms
- **Performance**: 3.7x faster than before, beats 4s industry average
- **Approach**: Direct Rust calls to SWC bundler (bypassed V8 isolate overhead)

## Goal
- **Target**: 0.25-0.5s (match Bun/Rolldown)
- **Strategy**: Custom parallel bundler with full control

## Architecture Design

### Phase 1: Parallel Module Discovery
```
Entry File
    ↓
[Worker Pool - N cores]
    ├─ Worker 1: Parse imports → Find deps → Queue new modules
    ├─ Worker 2: Parse imports → Find deps → Queue new modules
    └─ Worker N: Parse imports → Find deps → Queue new modules
    ↓
Dependency Graph (all modules discovered)
```

**Data Structures:**
- `DashMap<PathBuf, ParsedModule>` - Thread-safe module cache
- `crossbeam::channel` - Work queue for module paths
- `AtomicBool` - Termination signal when queue empty

### Phase 2: Parallel Parsing & Transformation
```
Discovered Modules
    ↓
[Worker Pool - Spawn tasks per module]
    ├─ Task 1: Read file → Parse → Transform (TS/JSX) → Cache
    ├─ Task 2: Read file → Parse → Transform (TS/JSX) → Cache
    └─ Task N: Read file → Parse → Transform (TS/JSX) → Cache
    ↓
Transformed Module Cache
```

**Key Optimizations:**
- Use `tokio::task::spawn_blocking` for CPU-intensive parsing
- Each task gets its own `SourceMap` (avoid Send/Sync issues)
- Parse files in batches to avoid overwhelming filesystem

### Phase 3: Dependency Resolution & Bundling
```
Transformed Modules
    ↓
Topological Sort (dependency order)
    ↓
Link & Concatenate
    ↓
Output Bundle
```

## Technical Challenges & Solutions

### Challenge 1: SWC's SourceMap isn't Send
**Solution:** Create a new `SourceMap` per task, merge at end

### Challenge 2: Progressive dependency discovery
**Solution:** Two-pass approach:
1. Quick scan for imports (parallel)
2. Full parse & transform (parallel)

### Challenge 3: Module resolution (node_modules traversal)
**Solution:** Cache resolution results in DashMap

### Challenge 4: Circular dependencies
**Solution:** Track visiting set, detect cycles early

## Implementation Steps

### Step 1: Extract Module Scanner (Week 1)
- Fast import scanner (no full parse)
- Parallel file reading
- Build dependency graph

### Step 2: Parallel Parser (Week 2)
- tokio task pool
- Per-task SourceMap
- Transform pipeline

### Step 3: Smart Bundler (Week 3)
- Topological sort
- Code generation
- Module wrapping

### Step 4: Optimizations (Week 4)
- Module resolution cache
- Persistent disk cache
- Incremental builds

## Benchmarks to Beat

| Tool | Time (10K components) | Approach |
|------|----------------------|----------|
| **Bun** | 0.25s | Custom Zig bundler, parallel-first |
| **Rolldown** | 0.5s | Rust + oxc, uses rayon |
| **Most bundlers** | ~4s | Various (Webpack, Parcel, etc.) |
| **Deka (current)** | 1.4-2.8s | SWC bundler, direct Rust calls |
| **Deka (target)** | 0.3-0.5s | Custom parallel bundler |

## Why Build Our Own?

1. **Independence** - Not tied to oxc/Rolldown development
2. **Control** - Optimize for Deka's specific use cases
3. **Learning** - Deep understanding of bundling internals
4. **Innovation** - Can experiment with novel approaches
5. **Integration** - Tight integration with Deka runtime features

## Future Enhancements

- **Incremental bundling** - Only rebuild changed modules
- **Persistent cache** - Disk cache across builds
- **Smart code splitting** - Automatic chunk optimization
- **Tree shaking** - Dead code elimination
- **Minification** - Integrated terser-like minifier
- **Source maps** - Full source map support

## References

- [SWC Bundler](https://github.com/swc-project/swc/tree/main/crates/swc_bundler)
- [oxc Bundler](https://github.com/oxc-project/oxc)
- [Rolldown](https://github.com/rolldown/rolldown)
- [Bun Bundler](https://bun.sh/docs/bundler)

## Current Status

✅ **Phase 0 Complete**: Optimized SWC integration (1.4-2.8s)
🔄 **Phase 1 In Progress**: Architecture design
⏳ **Phase 2 Pending**: Implementation
⏳ **Phase 3 Pending**: Benchmarking & optimization
