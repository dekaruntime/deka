# Desktop Compilation

The Deka desktop compilation system transforms web applications into native desktop applications using embedded WebView technology. This allows you to write your app once using web technologies (HTML, CSS, JavaScript) and deploy it as a native desktop app on macOS, Windows, and Linux.

## Overview

Desktop compilation combines three key technologies:

1. **VFS (Virtual File System)** - Embeds all your app files (HTML, CSS, JS, images) into the binary
2. **WebView** - Native web rendering engine (WebKit on macOS, WebView2 on Windows, WebKitGTK on Linux)
3. **Deka Runtime** - Serves your app and handles backend logic

The result is a **single native application** that runs without requiring a browser or external dependencies.

## Quick Start

```bash
# Basic desktop compilation
deka compile --desktop

# Create macOS .app bundle
deka compile --desktop --bundle

# Specify app name
deka compile --desktop --bundle --name "MyApp"

# With custom window settings (via deka.json)
deka compile --desktop --bundle
```

Output:
- macOS: `MyApp.app` (double-click to launch)
- Windows: `MyApp.exe`
- Linux: `myapp` binary

## Architecture

### Component Stack

```
┌─────────────────────────────────────────────┐
│          Native Desktop Window              │
│        (wry + tao window manager)           │
├─────────────────────────────────────────────┤
│           WebView Renderer                  │
│    (WebKit/WebView2/WebKitGTK)             │
├─────────────────────────────────────────────┤
│          HTTP Server Layer                  │
│       (serves from localhost)               │
├─────────────────────────────────────────────┤
│         Deka Runtime + Handler              │
│      (your JavaScript/TypeScript)           │
├─────────────────────────────────────────────┤
│        VFS (Virtual File System)            │
│   (embedded HTML, CSS, JS, assets)          │
└─────────────────────────────────────────────┘
```

### Request Flow

```
┌──────────┐
│  User    │
│  Click   │
└────┬─────┘
     │
     ▼
┌────────────────┐
│   WebView      │ http://localhost:RANDOM_PORT/
│   Renderer     │
└────┬───────────┘
     │
     ▼
┌────────────────┐
│  HTTP Server   │ Handle request
│  (localhost)   │
└────┬───────────┘
     │
     ▼
┌────────────────┐
│  Your Handler  │ export default { fetch(req) {} }
│  (JavaScript)  │
└────┬───────────┘
     │
     ▼
┌────────────────┐
│  VFS Lookup    │ fs.readFileSync('./index.html')
│  (cache.rs)    │
└────┬───────────┘
     │
     ▼
┌────────────────┐
│  Response      │ HTML/CSS/JSON returned
└────────────────┘
```

## Compilation Process

### Phase 1: File Discovery

```rust
// Scan project directory for files to embed
for entry in WalkDir::new(project_dir) {
    if should_include(entry) {
        files.push(entry);
    }
}

// Files included:
// - HTML, CSS, JS, JSON
// - Images (PNG, JPG, SVG, etc.)
// - Fonts (WOFF, WOFF2, TTF, OTF)
// - Your handler.js entry point
```

### Phase 2: VFS Creation

```rust
let mut vfs = VfsBuilder::new();

for file in files {
    let content = fs::read(&file)?;
    let relative_path = file.strip_prefix(project_dir)?;
    vfs.add_file(relative_path, content);
}

let vfs_bytes = vfs.build()?; // Serialized VFS
```

### Phase 3: Binary Embedding

```rust
// Take the runtime binary as template
let runtime_binary = include_bytes!("../../target/release/cli");

// Embed VFS into the binary
let mut output = Vec::new();
output.extend_from_slice(runtime_binary);

// Add VFS data
output.extend_from_slice(&vfs_bytes);

// Add metadata footer (offset, size, magic bytes)
let metadata = VfsMetadata {
    offset: runtime_binary.len() as u64,
    size: vfs_bytes.len() as u64,
    magic: VFS_MAGIC,
};
output.extend_from_slice(&bincode::serialize(&metadata)?);
```

### Phase 4: Platform Packaging

#### macOS (.app bundle)

When using `--bundle` on macOS, creates a proper application bundle:

```
MyApp.app/
├── Contents/
│   ├── Info.plist          # App metadata (name, version, identifier)
│   ├── MacOS/
│   │   └── MyApp           # The executable binary (runtime + VFS)
│   ├── Resources/          # App icon, assets (future)
│   └── PkgInfo             # Package type identifier
```

**Info.plist** structure:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
    "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>MyApp</string>
    <key>CFBundleDisplayName</key>
    <string>MyApp</string>
    <key>CFBundleIdentifier</key>
    <string>com.yourcompany.myapp</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundleExecutable</key>
    <string>MyApp</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
</dict>
</plist>
```

#### Windows (.exe)

Single executable with embedded resources:
```
MyApp.exe
  ├── Runtime binary
  ├── VFS data
  └── Windows resources (icon, manifest)
```

#### Linux (AppImage/binary)

```
myapp
  ├── Runtime binary
  └── VFS data

# Or AppImage format:
MyApp-x86_64.AppImage
  ├── AppRun
  ├── myapp.desktop
  ├── icon.png
  └── usr/bin/myapp
```

## Runtime Initialization

### Desktop Mode Detection

The binary detects it's running in desktop mode via environment or embedded flag:

```rust
// crates/runtime/src/main.rs
fn main() -> Result<()> {
    // Check if VFS is embedded
    let vfs = VfsProvider::from_embedded()?;

    if vfs.is_some() && is_desktop_mode() {
        // Desktop mode - start webview
        serve_desktop(vfs.unwrap())?;
    } else if vfs.is_some() {
        // Server mode - start HTTP server
        serve_vfs(vfs.unwrap())?;
    } else {
        // Dev mode - read from filesystem
        serve_dev()?;
    }
}
```

### Window Creation

```rust
// crates/runtime/src/desktop.rs
pub fn serve_desktop(vfs: VfsProvider) -> Result<()> {
    // 1. Start HTTP server on random port
    let port = find_available_port()?;
    let server = start_http_server(port, vfs)?;

    // 2. Create window with webview
    let window = WindowBuilder::new()
        .with_title("MyApp")
        .with_inner_size(LogicalSize::new(1200, 800))
        .build()?;

    // 3. Create webview pointing to localhost
    let webview = WebViewBuilder::new(window)?
        .with_url(&format!("http://localhost:{}", port))?
        .build()?;

    // 4. Run event loop
    webview.run();
}
```

### VFS Mounting

```rust
// Extract VFS from embedded data
pub fn from_embedded() -> Result<VfsProvider> {
    let exe_path = std::env::current_exe()?;
    let exe_data = std::fs::read(exe_path)?;

    // Read metadata from end of file
    let metadata_size = size_of::<VfsMetadata>();
    let metadata_start = exe_data.len() - metadata_size;
    let metadata: VfsMetadata = bincode::deserialize(
        &exe_data[metadata_start..]
    )?;

    // Verify magic bytes
    if metadata.magic != VFS_MAGIC {
        return Err("Not a VFS binary");
    }

    // Extract VFS data
    let vfs_start = metadata.offset as usize;
    let vfs_end = vfs_start + metadata.size as usize;
    let vfs_data = &exe_data[vfs_start..vfs_end];

    // Deserialize into file map
    let files: HashMap<String, Vec<u8>> =
        bincode::deserialize(vfs_data)?;

    Ok(VfsProvider { files })
}
```

## Configuration

### deka.json

Configure your desktop app via `deka.json` in your project root:

```json
{
  "app": {
    "name": "MyAwesomeApp",
    "version": "1.0.0",
    "identifier": "com.mycompany.myapp",
    "description": "An amazing desktop app"
  },
  "window": {
    "title": "My Awesome App",
    "width": 1200,
    "height": 800,
    "minWidth": 800,
    "minHeight": 600,
    "resizable": true,
    "fullscreen": false,
    "decorations": true,
    "alwaysOnTop": false,
    "transparent": false,
    "center": true
  },
  "compile": {
    "entry": "handler.js",
    "bundle": true,
    "desktop": true,
    "icon": "./assets/icon.png"
  }
}
```

### Configuration Fields

#### App Configuration
- `name` - Application name (shown in menus, task manager)
- `version` - Semantic version (e.g., "1.0.0")
- `identifier` - Reverse domain notation (e.g., "com.company.app")
- `description` - App description

#### Window Configuration
- `title` - Window title bar text
- `width` / `height` - Initial window dimensions
- `minWidth` / `minHeight` - Minimum window size
- `maxWidth` / `maxHeight` - Maximum window size
- `resizable` - Allow window resizing (default: true)
- `fullscreen` - Start in fullscreen mode (default: false)
- `decorations` - Show title bar and borders (default: true)
- `alwaysOnTop` - Keep window above others (default: false)
- `transparent` - Transparent window background (default: false)
- `center` - Center window on screen at startup (default: true)

#### Compile Configuration
- `entry` - Entry point file (default: "handler.js")
- `bundle` - Create platform bundle (.app, .exe, AppImage)
- `desktop` - Enable desktop mode
- `icon` - Path to app icon file

## Handler Example

Your handler serves the UI and handles backend logic:

```javascript
// handler.js
const fs = globalThis.__dekaNodeFs;

export default {
  async fetch(request) {
    const url = new URL(request.url);

    // Serve HTML pages
    if (url.pathname === '/' || url.pathname === '/index.html') {
      return new Response(fs.readFileSync('./index.html', 'utf8'), {
        headers: { 'Content-Type': 'text/html' }
      });
    }

    // Serve static assets
    if (url.pathname === '/style.css') {
      return new Response(fs.readFileSync('./style.css', 'utf8'), {
        headers: { 'Content-Type': 'text/css' }
      });
    }

    // API endpoint
    if (url.pathname === '/api/data') {
      const data = { message: 'Hello from desktop app!' };
      return new Response(JSON.stringify(data), {
        headers: { 'Content-Type': 'application/json' }
      });
    }

    return new Response('Not found', { status: 404 });
  }
}
```

## Advanced Features

### Native API Access (Future)

Desktop apps will be able to access native OS features:

```javascript
// Future API (not yet implemented)
import { dialog, clipboard, notifications } from 'deka/desktop';

// File picker
const file = await dialog.open({
  filters: [{ name: 'Images', extensions: ['png', 'jpg'] }]
});

// Clipboard
await clipboard.writeText('Hello!');

// System notifications
await notifications.show({
  title: 'Update Available',
  body: 'Version 2.0 is ready to install'
});
```

### Multi-Window Support (Future)

```javascript
// Future API
import { Window } from 'deka/desktop';

// Open new window
const preferences = new Window({
  url: '/preferences',
  width: 600,
  height: 400,
  title: 'Preferences'
});
```

### System Tray (Future)

```javascript
// Future API
import { Tray } from 'deka/desktop';

const tray = new Tray({
  icon: './assets/tray-icon.png',
  menu: [
    { label: 'Show Window', click: () => window.show() },
    { label: 'Quit', click: () => app.quit() }
  ]
});
```

## Platform-Specific Details

### macOS

**Bundle Structure:**
- `.app` bundle is standard macOS application format
- Launches via Finder with proper icon and metadata
- No accessibility permissions needed (uses standard WebKit)

**Code Signing:**
```bash
# Sign the app (required for distribution)
codesign --force --deep --sign "Developer ID Application: Your Name" MyApp.app

# Verify signature
codesign --verify --deep --strict MyApp.app
```

**Notarization:**
```bash
# Create DMG for distribution
hdiutil create -volname "MyApp" -srcfolder MyApp.app -ov MyApp.dmg

# Notarize with Apple
xcrun notarytool submit MyApp.dmg \
  --apple-id "you@example.com" \
  --team-id "TEAMID" \
  --password "app-specific-password"
```

### Windows

**Executable Details:**
- Single `.exe` file
- Uses WebView2 (Microsoft Edge WebView)
- Requires WebView2 runtime (included in Windows 11, downloadable for Windows 10)

**Installer Creation:**
```powershell
# Use WiX Toolset or Inno Setup
# Creates MSI installer with proper registry entries
```

### Linux

**Dependencies:**
- WebKitGTK for web rendering
- GTK3 for window management

**Install on Ubuntu/Debian:**
```bash
sudo apt install webkit2gtk-4.0 libgtk-3-dev
```

**AppImage Creation:**
```bash
# Bundle as AppImage for distribution
appimagetool MyApp.AppDir MyApp-x86_64.AppImage
```

## Distribution

### macOS Distribution

1. **Direct Distribution** - Share the `.app` bundle
2. **DMG Distribution** - Create a disk image
3. **App Store** - Submit via App Store Connect (requires Apple Developer account)

### Windows Distribution

1. **Direct Distribution** - Share the `.exe`
2. **Installer** - Create MSI/NSIS installer
3. **Microsoft Store** - Submit via Partner Center

### Linux Distribution

1. **Direct Binary** - Share the executable
2. **AppImage** - Universal Linux package
3. **Snap/Flatpak** - Linux app stores
4. **Distribution Repos** - Package for apt/yum/pacman

## Debugging Desktop Apps

### Enable Developer Tools

```bash
# Set environment variable to enable DevTools
DEKA_DEVTOOLS=1 open MyApp.app

# Or in handler.js
export default {
  devtools: true,  // Enable in development builds
  async fetch(request) {
    // ...
  }
}
```

### View Logs

```bash
# macOS: Console.app or terminal
log show --predicate 'process == "MyApp"' --last 5m

# Windows: Event Viewer or OutputDebugString
DebugView.exe

# Linux: journalctl
journalctl -f -t myapp
```

### Network Debugging

The desktop app runs a local HTTP server. You can access it directly:

```bash
# Find the port
lsof -i -P | grep MyApp

# Access in browser for debugging
curl http://localhost:RANDOM_PORT/
```

## Performance Optimization

### Binary Size

Typical sizes:
- **Base runtime**: ~8-12 MB (Rust + V8)
- **VFS overhead**: Original file size (will add compression)
- **Example app**: 100 files (~2MB assets) = ~14MB total

Optimization strategies:
- Minify HTML/CSS/JS before compilation
- Optimize images (use WebP, compress PNGs)
- Remove unused assets
- Use `.dekaignore` to exclude dev files

### Startup Time

- **Cold start**: ~100-200ms (VFS extraction + window creation)
- **Warm start**: ~50-100ms (if OS caches binary)

Optimization:
- Keep VFS size reasonable (< 50MB for best performance)
- Use lazy loading for large assets
- Minimize handler startup code

### Memory Usage

- **Base memory**: ~50-80 MB (runtime + WebView)
- **Per VFS file**: Loaded into memory at startup
- **WebView**: Similar to browser tab (~50-200 MB depending on content)

## Comparison to Other Tools

| Feature | Deka Desktop | Electron | Tauri | NW.js |
|---------|--------------|----------|-------|-------|
| Bundle Size | ~14 MB | ~120 MB | ~15 MB | ~100 MB |
| Language | JS/TS | JS/TS | Rust + JS/TS | JS/TS |
| WebView | Native | Chromium | Native | Chromium |
| Backend | Deka Runtime | Node.js | Rust | Node.js |
| VFS | Built-in | asar | Custom | Custom |
| Startup | ~100ms | ~500ms | ~100ms | ~400ms |

## Troubleshooting

### "VFS not found" Error
```bash
# Verify VFS was embedded
strings MyApp.app/Contents/MacOS/MyApp | grep "VFS_MAGIC"

# Re-compile with --desktop flag
deka compile --desktop --bundle
```

### Files Not Loading
```javascript
// ❌ Wrong - absolute path
fs.readFileSync('/index.html')

// ✅ Correct - relative path
fs.readFileSync('./index.html')
```

### Window Doesn't Appear
```bash
# Check if server started
lsof -i -P | grep MyApp

# Check logs for errors
log show --predicate 'process == "MyApp"' --last 1m
```

### WebView Not Rendering
- **macOS**: Ensure macOS 10.15+ (uses WKWebView)
- **Windows**: Install WebView2 runtime
- **Linux**: Install webkit2gtk-4.0

## Future Enhancements

Planned features:
- [ ] Hot reload in development mode
- [ ] Auto-update system
- [ ] Native menus and shortcuts
- [ ] Multi-window support
- [ ] System tray integration
- [ ] Native dialogs (file picker, alerts)
- [ ] Clipboard API
- [ ] Notifications API
- [ ] Deep linking support
- [ ] Custom URL protocols

## Conclusion

Deka's desktop compilation system provides a streamlined way to build native desktop applications using web technologies. By combining VFS, native WebView, and the Deka runtime, you get:

- **Single-file distribution** - One binary, no dependencies
- **Native performance** - Uses system WebView, not bundled Chromium
- **Small bundle size** - ~14MB vs 120MB+ for Electron
- **Fast startup** - Cold start in ~100ms
- **Transparent file access** - Same code works in dev and production
- **Cross-platform** - One codebase, multiple platforms

The system is designed to be invisible - write your app once with standard Node.js APIs, and it works seamlessly as both a web server and a desktop application.
