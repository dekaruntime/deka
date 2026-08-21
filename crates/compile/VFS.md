# VFS: Virtual File System

The Deka VFS (Virtual File System) is an embedded file system that allows compiled applications to bundle static assets directly into the binary. It provides transparent file access across all runtime modes without requiring users to write mode-specific code.

## Overview

VFS enables "compile once, run anywhere" by embedding project files (HTML, CSS, JS, images, etc.) directly into the compiled binary. The same code works seamlessly whether running in:

- **Dev server mode** (`deka serve`) - reads from filesystem
- **Desktop bundled app** (`.app` bundle) - reads from embedded VFS
- **Edge/serverless deployment** - reads from embedded VFS
- **Docker containers** - reads from embedded VFS

## Architecture

### Two-Layer System

```
┌─────────────────────────────────────────┐
│  User Code (JavaScript/TypeScript)      │
│  const fs = globalThis.__dekaNodeFs     │
│  fs.readFileSync('./index.html')        │
└─────────────────┬───────────────────────┘
                  │
                  │ (Node.js compatible API)
                  │
┌─────────────────▼───────────────────────┐
│  Runtime Layer (Rust)                   │
│  ┌─────────────────────────────────┐   │
│  │ cache.rs - VFS Resolution       │   │
│  │ • Check VFS first               │   │
│  │ • Fallback to filesystem        │   │
│  │ • DNS-style lookup              │   │
│  └─────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

### DNS-Style Resolution

Similar to how DNS resolution checks `/etc/hosts` before querying DNS servers, VFS uses a **VFS-first, filesystem-fallback** strategy:

```rust
// Simplified conceptual flow
fn resolve_file(path: &str) -> Result<Vec<u8>> {
    // 1. Check VFS first (if mounted)
    if let Some(vfs_content) = VFS_CACHE.get(path) {
        return Ok(vfs_content);
    }

    // 2. Fallback to filesystem
    std::fs::read(path)
}
```

This makes the abstraction **completely invisible** to user code - the same `fs.readFileSync('./index.html')` works everywhere.

## Implementation

### VFS Creation (Compile Time)

When compiling with `deka compile --desktop`, the compiler:

1. **Scans project directory** for static assets
2. **Embeds files** into a binary blob
3. **Generates metadata** (file paths, sizes, offsets)
4. **Injects VFS** into the compiled binary

```rust
// crates/compile/src/vfs_generator.rs
pub fn generate_vfs(project_dir: &Path) -> Result<Vec<u8>> {
    let mut vfs = VfsBuilder::new();

    // Scan and embed all project files
    for entry in WalkDir::new(project_dir) {
        let path = entry.path();
        let content = std::fs::read(path)?;
        vfs.add_file(path, content);
    }

    vfs.build()
}
```

### VFS Runtime (Execution Time)

At runtime, the VFS is:

1. **Extracted** from the binary's embedded data section
2. **Mounted** into memory as a hash map
3. **Queried** transparently via the Node.js fs API

```rust
// crates/runtime/src/vfs_loader.rs
pub struct VfsProvider {
    files: HashMap<String, Vec<u8>>,
}

impl VfsProvider {
    pub fn from_embedded() -> Result<Self> {
        // Extract VFS from binary
        let vfs_data = include_bytes!("../embedded_vfs.bin");
        let files = deserialize_vfs(vfs_data)?;
        Ok(Self { files })
    }

    pub fn read(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|v| v.as_slice())
    }
}
```

### Node.js API Integration

The VFS integrates with the Node.js-compatible filesystem API exposed to JavaScript:

```javascript
// User code - works everywhere
const fs = globalThis.__dekaNodeFs;

// In dev mode: reads from actual filesystem
// In VFS mode: reads from embedded VFS
// Users don't need to know which!
const html = fs.readFileSync('./index.html', 'utf8');
```

Behind the scenes (`crates/modules_js/src/modules/deka/fs.rs`):

```rust
#[op2]
#[buffer]
pub fn op_read_file(#[string] path: String) -> Result<Vec<u8>, AnyError> {
    // VFS-aware resolution happens in cache.rs
    cache::read_file_with_vfs_fallback(&path)
}
```

## Usage Patterns

### Basic File Reading

```javascript
const fs = globalThis.__dekaNodeFs;

// Read text files
const html = fs.readFileSync('./index.html', 'utf8');
const json = JSON.parse(fs.readFileSync('./config.json', 'utf8'));

// Read binary files
const image = fs.readFileSync('./logo.png');

// Check if file exists
const exists = fs.existsSync('./optional-config.json');
```

### Web Server Example

```javascript
const fs = globalThis.__dekaNodeFs;

export default {
  async fetch(request) {
    const url = new URL(request.url);

    // Serve static files - works in dev AND production
    if (url.pathname === '/') {
      return new Response(fs.readFileSync('./index.html', 'utf8'), {
        headers: { 'Content-Type': 'text/html' }
      });
    }

    if (url.pathname === '/style.css') {
      return new Response(fs.readFileSync('./style.css', 'utf8'), {
        headers: { 'Content-Type': 'text/css' }
      });
    }

    return new Response('Not found', { status: 404 });
  }
}
```

### Desktop App Example

```javascript
// Same code works in dev server and compiled .app
const fs = globalThis.__dekaNodeFs;

function loadConfig() {
  if (fs.existsSync('./config.json')) {
    return JSON.parse(fs.readFileSync('./config.json', 'utf8'));
  }
  return { theme: 'dark', port: 3000 };
}

export default {
  async fetch(request) {
    const config = loadConfig();
    return new Response(JSON.stringify(config), {
      headers: { 'Content-Type': 'application/json' }
    });
  }
}
```

## Path Resolution

### Supported Path Formats

The VFS resolver supports these path formats:

- **Relative paths**: `./index.html`, `../config.json`
- **Absolute paths**: `/Users/you/project/file.txt` (dev mode only)
- **Normalized paths**: Automatically handles `./`, `../`, and redundant slashes

### Path Normalization

All paths are normalized before VFS lookup:

```rust
// ./static/../index.html -> index.html
// ./foo/./bar.css -> foo/bar.css
// static/images//logo.png -> static/images/logo.png
```

### Working Directory

In compiled apps, the working directory is the VFS root (typically the project root at compile time). All relative paths resolve from there:

```javascript
// If compiled from /Users/you/myapp/
fs.readFileSync('./index.html')  // Reads /Users/you/myapp/index.html from VFS
```

## File Embedding Rules

### What Gets Embedded

By default, these files are embedded in the VFS:

- ✅ Static assets (`.html`, `.css`, `.js`, `.json`)
- ✅ Images (`.png`, `.jpg`, `.svg`, `.gif`, `.webp`)
- ✅ Fonts (`.woff`, `.woff2`, `.ttf`, `.otf`)
- ✅ Media (`.mp4`, `.webm`, `.mp3`, `.wav`)
- ✅ Documents (`.pdf`, `.txt`, `.md`)

### What Gets Excluded

These are automatically excluded:

- ❌ `node_modules/`
- ❌ `.git/`
- ❌ Hidden files (`.env`, `.gitignore`)
- ❌ Build artifacts (`target/`, `dist/`, `.build/`)
- ❌ Temp files (`.tmp`, `.cache`)

### Custom Exclusions

Use `.dekaignore` to exclude additional files:

```
# .dekaignore
*.log
test-fixtures/
*.tmp
private/
```

## Performance Characteristics

### Memory Usage

VFS files are loaded into memory at startup:

- **Small apps** (< 10MB assets): Negligible overhead (~20ms startup)
- **Medium apps** (10-50MB assets): ~50-100ms startup
- **Large apps** (50-100MB assets): ~200-500ms startup

### File Access Speed

Once loaded, VFS file access is **faster than filesystem** because it's pure memory access:

- **Filesystem read**: ~1-5ms per file (disk I/O)
- **VFS read**: ~0.001ms per file (memory access)

This makes VFS ideal for serving static assets in production.

### Binary Size

VFS adds to binary size:

- **Base runtime**: ~8-12MB
- **Per file**: Original file size (no compression yet)
- **Metadata overhead**: ~100 bytes per file

Example:
- 100 HTML/CSS/JS files (~2MB) = 12MB + 2MB = 14MB binary
- 500 files with images (~20MB) = 12MB + 20MB = 32MB binary

## Development Workflow

### Recommended Workflow

1. **Develop with `deka serve`** (filesystem mode)
   - Fast iteration
   - No compilation needed
   - Hot reload (future)

2. **Test with `deka compile --desktop`** (VFS mode)
   - Verify VFS paths work
   - Test bundled app behavior
   - Check binary size

3. **Deploy compiled binary**
   - Single file distribution
   - No external dependencies
   - Fast production startup

### Debugging VFS Issues

If files aren't loading from VFS:

1. **Check the VFS was embedded**:
   ```bash
   strings MyApp.app/Contents/MacOS/MyApp | grep "VFS_MAGIC"
   ```

2. **List embedded files** (introspection endpoint):
   ```bash
   curl http://localhost:8530/_deka/vfs
   ```

3. **Check file paths** match exactly:
   ```javascript
   // ❌ Wrong - leading slash
   fs.readFileSync('/index.html')

   // ✅ Correct - relative path
   fs.readFileSync('./index.html')
   ```

## Implementation Files

Key files in the VFS implementation:

### Compilation
- `crates/compile/src/vfs_generator.rs` - VFS creation and embedding
- `crates/compile/src/desktop.rs` - Desktop compilation with VFS

### Runtime
- `crates/runtime/src/vfs_loader.rs` - VFS extraction and mounting
- `crates/runtime/src/desktop.rs` - Desktop app VFS initialization
- `crates/modules_js/src/modules/deka/cache.rs` - VFS-aware file resolution
- `crates/modules_js/src/modules/deka/fs.rs` - File ops implementation

### API Surface
- `crates/modules_js/src/modules/deka/deka.js` - Node.js fs API exposed as `globalThis.__dekaNodeFs`

## Comparison to Other Runtimes

### Deka VFS vs Bun

Deka's approach is directly inspired by Bun:

| Feature | Deka | Bun |
|---------|------|-----|
| Transparent API | ✅ Same code, all modes | ✅ Same code, all modes |
| Node.js compatible | ✅ `globalThis.__dekaNodeFs` | ✅ `require('fs')` / `import fs` |
| VFS-first resolution | ✅ DNS-style fallback | ✅ Automatic |
| Single binary | ✅ Desktop apps | ✅ `bun build --compile` |
| Zero config | ✅ Just works | ✅ Just works |

### Deka VFS vs Deno

| Feature | Deka | Deno |
|---------|------|------|
| API transparency | ✅ Invisible | ❌ Requires `Deno.readFileSync` |
| Mode awareness | ✅ Automatic | ❌ Manual checks needed |
| Single binary | ✅ VFS embedded | ✅ But separate asset handling |

### Deka VFS vs Node.js + pkg

| Feature | Deka | Node.js + pkg |
|---------|------|---------------|
| Built-in | ✅ Native support | ❌ Third-party tool |
| API | ✅ Standard fs | ⚠️ Special `fs` handling |
| Performance | ✅ Fast (memory) | ⚠️ Slower (extraction) |

## Future Enhancements

Planned improvements to the VFS system:

### Compression
- [ ] Compress VFS blob with zstd/brotli
- [ ] Decompress on-demand or at startup
- [ ] ~50-70% size reduction expected

### Lazy Loading
- [ ] Load large files on first access
- [ ] Keep small files in memory
- [ ] Reduce startup time for large apps

### Write Support
- [ ] Allow runtime file creation
- [ ] Persist to user data directory
- [ ] Overlay filesystem (VFS + local writes)

### Hot Reload
- [ ] Detect file changes in dev mode
- [ ] Trigger handler reload
- [ ] Maintain state across reloads

## Conclusion

The VFS system provides a **zero-config, transparent** way to bundle static assets with your application. By using standard Node.js APIs (`globalThis.__dekaNodeFs`), your code automatically works across all deployment modes without any changes.

The DNS-style resolution (VFS-first, filesystem-fallback) ensures fast production performance while maintaining easy development workflows. This is the same approach used by Bun, making Deka apps portable and production-ready out of the box.
