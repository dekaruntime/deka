# Incremental Builds Design

## Overview

Incremental builds allow fast partial rebuilds by only recompiling changed modules and their dependents, while reusing cached modules for everything else.

**Target Performance:**
- First build: ~680ms (parallel bundler)
- Single file change: **50-200ms** (only rebuild changed + dependents)
- No changes: **10ms** (full cache hit)

## Current State vs Incremental

### Current (Entry-Level Cache)

```
┌─────────────────────────────────────────────────────┐
│ Entry File: src/index.jsx                          │
│                                                     │
│ Cache Key: SHA256(entry path)                      │
│ Cache Value: Entire bundled output                 │
│                                                     │
│ Invalidation: Entry file mtime                     │
└─────────────────────────────────────────────────────┘

Problem: If you change any imported module, entire
         bundle must be rebuilt from scratch
```

### Incremental (Module-Level Cache)

```
┌──────────────────────────────────────────────────────┐
│ Each Module Cached Individually                      │
│                                                       │
│ src/index.jsx          → Cache Entry A               │
│ src/components/App.jsx → Cache Entry B               │
│ src/utils/helpers.js   → Cache Entry C               │
│ ...                                                   │
│                                                       │
│ When src/utils/helpers.js changes:                   │
│   1. Rebuild helpers.js (changed)                    │
│   2. Find modules that import helpers.js             │
│   3. Rebuild those modules (dependents)              │
│   4. Reuse cache for everything else                 │
└──────────────────────────────────────────────────────┘

Benefit: Only rebuild what's necessary
```

## Architecture

### 1. Module-Level Cache Structure

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModule {
    /// Module path
    pub path: PathBuf,

    /// Source code content
    pub source: String,

    /// File modification time
    pub mtime: SystemTime,

    /// Content hash (SHA256 of source)
    pub content_hash: String,

    /// Transformed/transpiled code
    pub transformed_code: String,

    /// Direct dependencies (modules this imports)
    pub dependencies: Vec<PathBuf>,

    /// Reverse dependencies (modules that import this)
    /// NOTE: Computed at build time, not stored
    pub reverse_dependencies: Vec<PathBuf>,
}
```

**Cache Storage:**
```
~/.config/deka/bundler/incremental/
  ├── modules/
  │   ├── <hash1>.json  # src/index.jsx
  │   ├── <hash2>.json  # src/App.jsx
  │   └── <hash3>.json  # src/utils.js
  └── graph.json        # Dependency graph metadata
```

### 2. Dependency Graph

Track both forward and reverse dependencies:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// Forward edges: module -> [modules it imports]
    pub dependencies: HashMap<PathBuf, HashSet<PathBuf>>,

    /// Reverse edges: module -> [modules that import it]
    pub reverse_dependencies: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl DependencyGraph {
    /// Add a dependency edge
    pub fn add_edge(&mut self, from: PathBuf, to: PathBuf) {
        self.dependencies.entry(from.clone())
            .or_insert_with(HashSet::new)
            .insert(to.clone());

        self.reverse_dependencies.entry(to)
            .or_insert_with(HashSet::new)
            .insert(from);
    }

    /// Find all modules affected by a change (transitive closure)
    pub fn get_affected(&self, changed: &PathBuf) -> HashSet<PathBuf> {
        let mut affected = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(changed.clone());
        affected.insert(changed.clone());

        while let Some(module) = queue.pop_front() {
            if let Some(rev_deps) = self.reverse_dependencies.get(&module) {
                for dep in rev_deps {
                    if affected.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        affected
    }
}
```

### 3. Change Detection

```rust
pub struct ChangeDetector {
    cache: ModuleCache,
    graph: DependencyGraph,
}

impl ChangeDetector {
    /// Detect which modules have changed since last build
    pub fn detect_changes(&self, modules: &[PathBuf]) -> ChangeSet {
        let mut changed = HashSet::new();
        let mut unchanged = HashSet::new();

        for path in modules {
            if let Some(cached) = self.cache.get(path) {
                // Check if file has been modified
                if let Ok(metadata) = fs::metadata(path) {
                    if let Ok(mtime) = metadata.modified() {
                        if mtime == cached.mtime {
                            // Content hash double-check
                            if let Ok(source) = fs::read_to_string(path) {
                                let hash = hash_file_content(&source);
                                if hash == cached.content_hash {
                                    unchanged.insert(path.clone());
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            // Either not in cache or modified
            changed.insert(path.clone());
        }

        ChangeSet { changed, unchanged }
    }

    /// Get full set of modules that need rebuilding
    pub fn get_rebuild_set(&self, changes: &ChangeSet) -> HashSet<PathBuf> {
        let mut rebuild = HashSet::new();

        for changed_module in &changes.changed {
            // Add changed module itself
            rebuild.insert(changed_module.clone());

            // Add all modules affected by this change (transitive)
            let affected = self.graph.get_affected(changed_module);
            rebuild.extend(affected);
        }

        rebuild
    }
}
```

## Build Flow

### First Build (Cold)

```
1. DISCOVERY
   ┌────────────────────────────────────┐
   │ Discover all modules from entry    │
   │ (using parallel bundler)           │
   └────────────────┬───────────────────┘
                    │
                    ▼
2. TRANSFORM
   ┌────────────────────────────────────┐
   │ Transform all modules in parallel  │
   │ (parse, transpile, extract deps)   │
   └────────────────┬───────────────────┘
                    │
                    ▼
3. CACHE & GRAPH
   ┌────────────────────────────────────┐
   │ Store each module in cache         │
   │ Build dependency graph             │
   │ Save graph to disk                 │
   └────────────────┬───────────────────┘
                    │
                    ▼
4. BUNDLE
   ┌────────────────────────────────────┐
   │ Concatenate in dependency order    │
   │ Generate final output              │
   └────────────────────────────────────┘

Time: ~680ms (same as current parallel bundler)
```

### Incremental Build (Warm)

```
1. LOAD CACHE & GRAPH
   ┌────────────────────────────────────┐
   │ Load dependency graph from disk    │
   │ Load module cache                  │
   └────────────────┬───────────────────┘
                    │
                    ▼
2. DETECT CHANGES
   ┌────────────────────────────────────┐
   │ Check mtime for all modules        │
   │ Identify changed modules           │
   └────────────────┬───────────────────┘
                    │
                    ▼
3. COMPUTE REBUILD SET
   ┌────────────────────────────────────┐
   │ Find modules affected by changes   │
   │ (transitive reverse dependencies)  │
   └────────────────┬───────────────────┘
                    │
                    ▼
4. SELECTIVE REBUILD
   ┌────────────────────────────────────┐
   │ Transform only modules in rebuild  │
   │ set (in parallel)                  │
   │ Reuse cached code for others       │
   └────────────────┬───────────────────┘
                    │
                    ▼
5. UPDATE CACHE & GRAPH
   ┌────────────────────────────────────┐
   │ Update cache entries for rebuilt   │
   │ modules                             │
   │ Update dependency graph if needed  │
   └────────────────┬───────────────────┘
                    │
                    ▼
6. BUNDLE
   ┌────────────────────────────────────┐
   │ Concatenate all modules            │
   │ (mix of cached + newly built)      │
   └────────────────────────────────────┘

Time: 50-200ms (depends on change scope)
```

## Performance Scenarios

### Scenario 1: No Changes
- Load cache + graph: ~5ms
- Detect changes: ~5ms
- **Total: ~10ms**

### Scenario 2: Single Leaf Module Changed
```
Example: Change src/utils/formatDate.js
  - Used by: src/components/DatePicker.jsx
  - Which is used by: src/components/App.jsx
  - Which is used by: src/index.jsx

Rebuild set: [formatDate.js, DatePicker.jsx, App.jsx, index.jsx]
  - 4 modules out of 10,000
  - Transform: ~20ms (parallel)
  - Bundle: ~30ms
  - **Total: ~50ms**
```

### Scenario 3: Core Utility Changed
```
Example: Change src/utils/api.js
  - Used by: 50 components
  - Which are used by: 100+ modules transitively

Rebuild set: 150 modules out of 10,000
  - Transform: ~80ms (parallel)
  - Bundle: ~60ms
  - **Total: ~140ms**
```

### Scenario 4: Entry File Changed
```
Example: Change src/index.jsx
  - Nothing depends on entry (it's the root)

Rebuild set: [index.jsx]
  - 1 module out of 10,000
  - Transform: ~5ms
  - Bundle: ~30ms
  - **Total: ~35ms**
```

## Implementation Plan

### Phase 1: Module-Level Cache
- Modify `ModuleCache` to store individual modules
- Key by module path hash instead of entry path
- Store all module metadata (deps, mtime, hash)

### Phase 2: Dependency Graph
- Build `DependencyGraph` during initial discovery
- Compute reverse dependencies
- Persist graph to disk

### Phase 3: Change Detection
- Implement `ChangeDetector`
- Check mtime for all known modules
- Compute transitive rebuild set

### Phase 4: Selective Rebuild
- Modify parallel bundler to accept "rebuild set"
- Skip transformation for cached modules
- Mix cached + new transformed code

### Phase 5: Integration
- Wire into `build.rs`
- Add environment variable: `DEKA_INCREMENTAL=1`
- Metrics logging for rebuild stats

## Cache Invalidation Strategies

### Smart Invalidation
```rust
// Config changes invalidate everything
if config_changed() {
    cache.clear();
}

// Dependency changes invalidate transitively
let affected = graph.get_affected(&changed_path);
for module in affected {
    cache.invalidate(module);
}

// Deleted files remove from graph
if !path.exists() {
    graph.remove_node(path);
    cache.remove(path);
}
```

### Cache Size Management
```rust
// Limit cache size (e.g., 1GB)
if cache.size() > MAX_CACHE_SIZE {
    // LRU eviction
    cache.evict_least_recently_used(0.2); // Evict 20%
}
```

## Metrics and Debugging

```
[incremental] loaded graph (20,002 modules)
[incremental] detected changes: 2 modules
[incremental] computed rebuild set: 15 modules (0.07%)
[incremental] cache hits: 19,987 / 20,002 (99.93%)
[incremental] transform time: 25ms
[incremental] bundle time: 35ms
build complete [60ms]
```

## Expected Impact

| Change Type | Modules Affected | Build Time | Improvement vs Full |
|-------------|------------------|------------|---------------------|
| No change | 0 | 10ms | 68x faster |
| Leaf module | 1-10 | 50ms | 13x faster |
| Mid-level | 10-100 | 140ms | 4.8x faster |
| Core utility | 100-500 | 280ms | 2.4x faster |
| Entry file | 1 | 35ms | 19x faster |

## Trade-offs

**Pros:**
- Massive speedup for partial changes
- Better developer experience (watch mode)
- Scales with codebase size (cache grows, but benefits increase)

**Cons:**
- More complex cache management
- Disk space for module cache (1-5MB per 10K modules)
- Dependency graph overhead (~100KB for 10K modules)
- Potential for cache corruption (need validation)

## Next Steps

1. Implement `DependencyGraph` struct
2. Modify `ModuleCache` for module-level storage
3. Add `ChangeDetector` logic
4. Update parallel bundler to support selective rebuild
5. Integrate with build command
6. Add metrics and logging
7. Test on 10K module benchmark
