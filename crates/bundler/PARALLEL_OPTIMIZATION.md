# Parallel Bundler Optimization Plan

## Current Performance Issue

The parallel bundler performs well on small/medium bundles (188ms vs Bun's 250ms) but has significant overhead at 10K+ modules (8.6s vs 3.7s standard bundler).

## Root Cause Analysis

### Critical Bottlenecks (in priority order):

#### 1. Worker Sleep/Polling (Line 109) - ~300-500ms overhead
```rust
tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
```
Workers busy-wait with 10ms sleeps when queue is empty. With 10K modules, this creates significant idle time.

**Fix:** Replace with channel-based signaling using `tokio::sync::mpsc`.

#### 2. Excessive Lock Contention - ~2-3s overhead
Hot path acquires locks multiple times per module:
- Read locks on `modules` + `processing` for EVERY dependency check
- Write lock on `pending` for EVERY dependency add
- Write lock on `modules` + `processing` for EVERY module completion

With 10K modules × 10 deps average = **100K+ lock acquisitions**.

**Fix:** Use channels for work distribution instead of shared `Arc<RwLock<VecDeque>>`.

#### 3. No Dependency Batching - ~500ms overhead
Dependencies added one-at-a-time with individual lock acquisitions:
```rust
for dep in &parsed.dependencies {
    pending.write().push_back(dep_path); // Lock per dependency!
}
```

**Fix:** Batch all dependencies and send as single message.

#### 4. Duplicate Dependency Resolution - ~200ms overhead
Dependencies are resolved twice:
- Once during discovery (line 138)
- Again during topological sort (line 393)

**Fix:** Store resolved dependency paths in `ParsedModule`.

#### 5. Inefficient Topological Sort - ~100-200ms overhead
DFS-based sort with repeated visits. O(N²) worst case.

**Fix:** Use Kahn's algorithm (O(N+E)).

## Optimization Strategy

### Phase 1: Channel-Based Architecture (Highest Impact)

Replace shared state (`Arc<RwLock<VecDeque>>`) with channels:

```rust
struct WorkMessage {
    path: PathBuf,
    dependencies: Vec<PathBuf>, // Pre-resolved!
}

// Workers receive work via channel
let (work_tx, mut work_rx) = tokio::sync::mpsc::unbounded_channel();

// Workers send results via channel
let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel();
```

**Benefits:**
- Eliminates sleep/polling (workers block on channel recv)
- Eliminates lock contention on work queue
- Natural backpressure mechanism
- Cleaner shutdown logic

**Expected Impact:** 2-3s improvement (60-70% of overhead)

### Phase 2: Dependency Batching

Send all dependencies as a batch instead of one-at-a-time:

```rust
// Before (one lock per dependency):
for dep in &dependencies {
    pending.write().push_back(dep);
}

// After (one message with all dependencies):
work_tx.send(WorkBatch { paths: dependencies })?;
```

**Expected Impact:** 300-500ms improvement

### Phase 3: Store Resolved Dependencies

```rust
pub struct ParsedModule {
    pub path: PathBuf,
    pub source: String,
    pub module: Module,
    pub dependencies: Vec<String>,         // Original specifiers
    pub resolved_dependencies: Vec<PathBuf>, // NEW: Resolved paths
}
```

Avoids re-resolving during topological sort.

**Expected Impact:** 200ms improvement

### Phase 4: Kahn's Algorithm for Topological Sort

```rust
fn sort_modules_kahn(&self, modules: &HashMap<PathBuf, ParsedModule>) -> Vec<PathBuf> {
    let mut in_degree: HashMap<PathBuf, usize> = HashMap::new();
    let mut queue = VecDeque::new();

    // Calculate in-degrees
    for module in modules.values() {
        for dep in &module.resolved_dependencies {
            *in_degree.entry(dep.clone()).or_insert(0) += 1;
        }
    }

    // Start with modules with no dependencies
    for path in modules.keys() {
        if in_degree.get(path).copied().unwrap_or(0) == 0 {
            queue.push_back(path.clone());
        }
    }

    // Process queue
    let mut sorted = Vec::new();
    while let Some(path) = queue.pop_front() {
        sorted.push(path.clone());

        if let Some(module) = modules.get(&path) {
            for dep in &module.resolved_dependencies {
                let degree = in_degree.get_mut(dep).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(dep.clone());
                }
            }
        }
    }

    sorted
}
```

**Expected Impact:** 100-200ms improvement

## Expected Results

| Optimization | Current | After Fix | Improvement |
|--------------|---------|-----------|-------------|
| Worker sleep/polling | ~400ms | ~0ms | -400ms |
| Lock contention | ~2500ms | ~200ms | -2300ms |
| Dependency batching | ~400ms | ~0ms | -400ms |
| Duplicate resolution | ~200ms | ~0ms | -200ms |
| Topological sort | ~200ms | ~50ms | -150ms |
| **TOTAL** | **8600ms** | **~3850ms** | **-3450ms** |

**Target:** ~2-3s for 10K module bundle (competitive with standard bundler, better for smaller bundles)

## Implementation Priority

1. **Phase 1** (Channel-based architecture) - 70% of gains
2. **Phase 2** (Dependency batching) - 15% of gains
3. **Phase 3** (Store resolved deps) - 10% of gains
4. **Phase 4** (Kahn's algorithm) - 5% of gains

Start with Phase 1 as it provides the most impact and simplifies the code.
