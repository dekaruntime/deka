# deka compile

Single-file executable compilation for Deka applications, inspired by `bun compile` and `deno compile`.

## Status: ✅ Phase 1 Complete - Binary Compilation Working!

The `deka compile` command is now functional and can create single-file executables. The VFS (Virtual File System) and binary embedding are complete and tested.

**What Works:**
- ✅ VFS creation with gzip compression
- ✅ File discovery and embedding
- ✅ Binary embedding (runtime + VFS + metadata)
- ✅ VFS extraction and verification
- ✅ Executable permissions set correctly
- ✅ Full integration tests passing

**What's Next:**
- Runtime needs to detect and mount embedded VFS on startup
- Execute code from embedded VFS instead of filesystem

**Try it now:**
```bash
# Create a handler.js file
echo 'export default { fetch() { return new Response("Hello!") } }' > handler.js

# Compile to single-file executable
deka compile

# Output: deka-app (174MB executable with full runtime + your code)
```

## Overview

The `compile` command bundles your Deka application (including the V8 runtime, PHP WASM runtime, dependencies, and user code) into a single portable executable binary. This brings the ease-of-use of JavaScript ecosystem compile features to PHP for the first time.

## Goals

1. **Single-file deployment** - One binary containing everything needed to run your app
2. **Cross-platform** - Compile for different target platforms (Linux, macOS, Windows)
3. **Zero dependencies** - No need to install Deka, PHP, or any runtime on target system
4. **Fast startup** - Pre-compiled, optimized for quick cold starts
5. **Production-ready** - Include all assets, modules, and dependencies

## Architecture

### Components

1. **Virtual File System (VFS)**
   - Embed user code, dependencies, and assets into the binary
   - Mount at runtime with read-only access
   - Support for both JS/TS and PHP files

2. **Runtime Bundling**
   - Include V8 isolate runtime (deno_core)
   - Include PHP WASM runtime (php.wasm)
   - Bundle all necessary Deka modules (`deka/router`, `deka/postgres`, etc.)

3. **Executable Generation**
   - Create platform-specific binary
   - Embed compressed VFS data
   - Self-extracting at startup
   - Preserve permissions and metadata

4. **Entry Point Resolution**
   - Detect main handler from project structure
   - Support explicit `--entrypoint` flag
   - Handle both HTTP server and CLI modes

### How It Works

```
┌─────────────────────────────────────────────────────────┐
│                    User's Application                   │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐   │
│  │ handler.js │  │  deps/     │  │    assets/     │   │
│  │ or app.php │  │            │  │                │   │
│  └────────────┘  └────────────┘  └────────────────┘   │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
                 ┌────────────────┐
                 │ deka compile   │
                 │                │
                 │ 1. Scan files  │
                 │ 2. Bundle deps │
                 │ 3. Create VFS  │
                 │ 4. Embed into  │
                 │    runtime     │
                 └────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│              Single Executable Binary                   │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │ Deka Runtime (Rust + V8 + PHP WASM)                │ │
│  ├────────────────────────────────────────────────────┤ │
│  │ Embedded VFS (compressed)                          │ │
│  │  - User code (JS/TS/PHP)                           │ │
│  │  - Dependencies (node_modules equivalent)          │ │
│  │  - Static assets                                   │ │
│  │  - Configuration                                   │ │
│  └────────────────────────────────────────────────────┘ │
│                                                          │
│  Binary runs standalone - no external dependencies      │
└─────────────────────────────────────────────────────────┘
```

### Build Process

1. **Discovery Phase**
   - Find entry point (handler.js, app.php, etc.)
   - Scan imports/requires to build dependency graph
   - Detect assets and static files

2. **Bundling Phase**
   - Transpile TypeScript to JavaScript (if needed)
   - Bundle dependencies
   - Optimize and minify code
   - Compress assets

3. **VFS Creation**
   - Create virtual filesystem structure
   - Compress file contents
   - Generate lookup tables for fast access

4. **Embedding Phase**
   - Take the Deka runtime binary as template
   - Embed VFS data into dedicated section
   - Add metadata (entry point, config, etc.)
   - Set executable permissions

5. **Output**
   - Single binary file (e.g., `my-app` or `my-app.exe`)
   - Optional: Cross-compile for other platforms

## Usage

```bash
# Compile current project
deka compile

# Specify entry point
deka compile --entry handler.js

# Specify output name
deka compile --output my-app

# Cross-compile for Linux
deka compile --target linux-x64

# Include specific directories
deka compile --include public,assets
```

## Implementation Plan

### Phase 1: Basic VFS ✅ COMPLETE
- [x] Create VFS data structure
- [x] Implement file embedding
- [x] Binary embedding with metadata footer
- [x] VFS extraction and verification
- [x] Test with simple JS handler
- [ ] Runtime VFS mount and execution (next step)

### Phase 2: PHP Support
- [ ] Include PHP WASM runtime
- [ ] Embed PHP files in VFS
- [ ] Test with PHP handler

### Phase 3: Dependency Bundling
- [ ] Scan imports/requires
- [ ] Bundle node_modules (for JS)
- [ ] Bundle composer deps (for PHP)

### Phase 4: Asset Handling
- [ ] Include static assets
- [ ] Support for binary files (images, etc.)
- [ ] Efficient compression

### Phase 5: Cross-compilation
- [ ] Support multiple target platforms
- [ ] Platform-specific binary templates
- [ ] Cross-platform testing

## Technical Details

### VFS Format

```rust
struct VFS {
    version: u32,
    entry_point: String,
    files: HashMap<PathBuf, FileEntry>,
}

struct FileEntry {
    content: Vec<u8>,  // Compressed
    metadata: FileMetadata,
}
```

### Binary Layout

```
┌─────────────────────────────┐
│ Deka Runtime Executable     │
│ (Rust binary with V8/WASM)  │
├─────────────────────────────┤
│ Metadata Section            │
│ - VFS offset                │
│ - VFS size                  │
│ - Entry point               │
│ - Version info              │
├─────────────────────────────┤
│ VFS Data (compressed)       │
│ - User code                 │
│ - Dependencies              │
│ - Assets                    │
└─────────────────────────────┘
```

## Similar Tools

- `bun compile` - JavaScript/TypeScript to executable
- `deno compile` - TypeScript/JavaScript to executable
- `pkg` (node) - Node.js to executable
- `PyInstaller` - Python to executable

## Why This Matters

This will be the **first time** that PHP developers can compile their applications into single-file executables with this level of ease. Combined with Deka's edge-compatible runtime, this opens up entirely new deployment patterns for PHP applications.
