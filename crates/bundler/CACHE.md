# Bundler Cache

## Overview

The Deka bundler includes a persistent module cache that dramatically improves build performance by storing bundled output and reusing it when source files haven't changed.

**Performance:**
- **Cached builds:** ~10ms (328x faster than uncached)
- **Uncached builds:** ~3s

The cache is **enabled by default** and uses file modification time (mtime) to detect when files have changed.

## How It Works

### Cache Strategy

The cache operates at the **entry file level** (Phase 1 implementation):

1. When you run `deka build src/index.jsx`, the bundler:
   - Computes a hash of the entry file path
   - Checks if a cached bundle exists for that file
   - Compares the file's modification time (mtime) with the cached version
   - If the file hasn't changed: returns cached bundle (~10ms)
   - If the file has changed: bundles from scratch and updates cache (~3s)

### Storage

**Location:** `~/.config/deka/bundler/cache/`

The cache uses a two-tier architecture:
- **Disk cache:** Persistent JSON files stored in the cache directory
- **Memory cache:** In-process HashMap for ultra-fast lookups within the same build session

**Cache entry structure:**
```rust
{
  "path": "/path/to/src/index.jsx",
  "source": "import React from 'react'...",
  "mtime": "2026-01-12T15:30:00Z",
  "content_hash": "a3f2e9...",
  "transformed_code": "bundled output...",
  "dependencies": []
}
```

### Cache Invalidation

The cache automatically invalidates when:
- The source file's modification time (mtime) changes
- The source file is deleted
- The cache file becomes corrupted (automatically deleted)

**Note:** Currently, the cache only tracks the entry file. If you modify an imported dependency, the cache won't invalidate automatically. This will be addressed in Phase 2 (incremental builds with dependency tracking).

### Cache Keys

Cache keys are generated using SHA256 hashing:
```rust
// Example: /Users/you/project/src/index.jsx
// Hashes to: 7f3a9e2b1c4d8a6f...
// Cache file: ~/.config/deka/bundler/cache/7f3a9e2b1c4d8a6f.json
```

Only the first 16 characters of the hash are used for shorter filenames.

## Usage

### Default Behavior (Cache Enabled)

Just run build commands normally:

```bash
deka build src/index.jsx

# First run:
#  [cache] MISS - bundling from scratch
#  build complete [3052ms]

# Second run:
#  [cache] HIT - using cached bundle
#  build complete [10ms]
```

### Disabling the Cache

Set the `DEKA_BUNDLER_CACHE` environment variable to `0` or `false`:

```bash
# Disable for a single build
DEKA_BUNDLER_CACHE=0 deka build src/index.jsx

# Disable for the session
export DEKA_BUNDLER_CACHE=0
deka build src/index.jsx
```

When disabled, no cache messages will appear and builds always run from scratch.

### Clearing the Cache

Use the `--clear-cache` flag to completely clear the cache:

```bash
deka build --clear-cache
```

This removes all cached bundles from `~/.config/deka/bundler/cache/`.

**When to clear the cache:**
- After upgrading the bundler (cache format changes)
- When you modify imported dependencies and the build doesn't pick up changes
- When troubleshooting build issues
- To reclaim disk space

### Checking Cache Status

Cache statistics are shown when the cache is enabled:

```bash
deka build src/index.jsx
# [cache] enabled (2 in memory, 5 on disk)
```

This shows:
- **In memory:** Number of entries in the current process's memory cache
- **On disk:** Total number of cached bundles in the cache directory

## Cache Lifecycle

```
┌─────────────────────────────────────────────────────┐
│ deka build src/index.jsx                            │
└────────────┬────────────────────────────────────────┘
             │
             ▼
    ┌────────────────────┐
    │ Initialize Cache   │
    │ (~/.config/deka/   │
    │  bundler/cache/)   │
    └────────┬───────────┘
             │
             ▼
    ┌────────────────────┐
    │ Hash entry path    │
    │ (SHA256)           │
    └────────┬───────────┘
             │
             ▼
    ┌────────────────────┐
    │ Check memory cache │
    └────┬───────────────┘
         │
         ├─ HIT ──► Validate mtime ──┬─ Valid ──► Return cached (10ms)
         │                            │
         │                            └─ Invalid ─► Remove from cache
         │
         └─ MISS ──► Check disk cache ──┬─ HIT ──► Validate mtime ──┬─ Valid ──► Load to memory ──► Return (10ms)
                                         │                            │
                                         │                            └─ Invalid ─► Delete file
                                         │
                                         └─ MISS ──► Bundle from scratch (3s) ──► Store to disk + memory
```

## Technical Implementation

### Code Structure

- **`crates/bundler/src/cache.rs`** - Core cache implementation
  - `ModuleCache` - Main cache manager
  - `CachedModule` - Serializable cache entry
  - `hash_file_content()` - SHA256 hashing utility

- **`crates/bundler/src/bundler.rs`** - Cache-aware bundling
  - `bundle_browser_assets_cached()` - Wrapper that checks cache before bundling

- **`crates/runtime/src/build.rs`** - Build command integration
  - Initializes `ModuleCache`
  - Handles `--clear-cache` flag

### Dependencies

```toml
serde = { version = "1.0", features = ["derive"] }
sha2 = "0.10"
```

### Example Usage in Code

```rust
use bundler::ModuleCache;

// Initialize cache
let mut cache = ModuleCache::new(None);

// Bundle with cache
let bundle = bundler::bundle_browser_assets_cached("./src/index.jsx", &mut cache)?;

// Clear cache
cache.clear()?;

// Get stats
let stats = cache.stats();
println!("Memory: {}, Disk: {}", stats.memory_count, stats.disk_count);
```

## Current Limitations (Phase 1)

1. **Entry file only** - Only caches the final bundle for the entry file, not individual modules
2. **No dependency tracking** - Changes to imported files don't invalidate the cache
3. **CSS not cached** - Only JavaScript bundles are cached currently
4. **No cross-platform cache** - Cache is local to each machine

## Future Enhancements (Phase 2+)

### Phase 2: Incremental Builds with Dependency Tracking

- Cache individual modules (not just entry file)
- Track import graph and dependencies
- Only rebuild changed modules + dependents
- Expected performance: ~50-200ms for partial changes

### Phase 3: Advanced Optimizations

- Shared cache across projects (monorepo support)
- Content-addressable storage (hash-based deduplication)
- Parallel module transformation
- Background cache warming

## Troubleshooting

### Cache not working?

Check that:
1. Cache is enabled (should see `[cache] enabled` message)
2. File permissions are correct on `~/.config/deka/bundler/cache/`
3. File system supports modification times (mtime)

### Stale cache entries?

If you modify imported dependencies but the build doesn't pick up changes:

```bash
# Option 1: Clear the cache
deka build --clear-cache

# Option 2: Touch the entry file to update its mtime
touch src/index.jsx
```

### Cache taking too much disk space?

Check cache size:
```bash
du -sh ~/.config/deka/bundler/cache/
```

Clear old entries:
```bash
deka build --clear-cache
```

Cache files are JSON and typically 1-5MB per entry depending on bundle size.

## Performance Comparison

| Scenario | Time | Notes |
|----------|------|-------|
| First build (cache MISS) | ~3s | Full bundle from scratch |
| Second build (cache HIT) | ~10ms | 328x faster |
| Build after touching entry file | ~3s | Cache invalidated |
| Build after changing imported file | ~10ms | **Warning:** Won't detect change (Phase 1 limitation) |
| Bun (uncached) | ~269ms | For comparison |
| Parallel bundler (mid-size) | ~188ms | Beats Bun, but has overhead at scale |

## See Also

- [OPTIMIZE.md](./OPTIMIZE.md) - Bundler optimization roadmap and benchmarks
- [PARALLEL_BUNDLER_ROADMAP.md](./PARALLEL_BUNDLER_ROADMAP.md) - Parallel bundler implementation notes
