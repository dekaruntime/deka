# Technical Deep Dive: Binary Appending & VFS Compilation

## Overview

This document explains the low-level technical details of how `deka compile` creates single-file executables by appending VFS data to binary files.

**TL;DR:** We copy the Deka binary and append user code to the end. The OS ignores the appended data, but our runtime can read it.

## What Is a Binary File?

A compiled executable is just **bytes in a file** with special headers:

```
┌─────────────────────────────────────┐
│ File: /usr/local/bin/deka           │
├─────────────────────────────────────┤
│ 0x00000000: 7F 45 4C 46 02 ...     │ ← ELF/Mach-O header
│ 0x00000100: 48 8B 45 F8 ...        │ ← Code section (.text)
│ 0x00002000: 00 01 02 03 ...        │ ← Data section (.data)
│ 0x00010000: [END OF EXECUTABLE]     │
└─────────────────────────────────────┘
```

**Key insight:** The OS only cares about the **header** which declares:
- Where code starts
- Where data starts
- How much memory to allocate
- Entry point address

Everything **after** those declared sections? **The OS ignores it!**

## How Binary Loading Works

When you run `./deka-app`:

```
1. OS reads ELF/Mach-O header
   ├─ "Code section: bytes 0x100-0x5000"
   ├─ "Data section: bytes 0x5000-0x10000"
   └─ "Entry point: 0x1A40"

2. OS loads those sections into memory
   ├─ Code → RAM address 0x400000
   ├─ Data → RAM address 0x600000
   └─ Ignores everything else!

3. OS jumps to entry point
   └─ Your program starts running
```

**The OS never reads past the declared sections!**

## The Append Trick

In `binary.rs`, we literally just copy bytes:

```rust
pub fn embed(
    &self,
    vfs_data: &[u8],
    entry_point: &str,
    output_path: &Path,
) -> Result<(), String> {
    // 1. Read the entire runtime binary
    let runtime_binary = fs::read(&self.runtime_binary_path)?;

    // Calculate where VFS will start (after the binary)
    let vfs_offset = runtime_binary.len() as u64;
    let vfs_size = vfs_data.len() as u64;

    // 2. Create metadata footer
    let metadata = BinaryMetadata::new(vfs_offset, vfs_size, entry_point.to_string());
    let metadata_bytes = metadata.to_bytes();

    // 3. Create output file
    let mut output_file = File::create(output_path)?;

    // 4. Write runtime binary (180MB)
    output_file.write_all(&runtime_binary)?;

    // 5. Append VFS data (1KB) - OS won't see this!
    output_file.write_all(vfs_data)?;

    // 6. Append metadata footer (40 bytes) - OS won't see this!
    output_file.write_all(&metadata_bytes)?;

    // 7. Set executable permissions
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(output_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output_path, perms)?;
    }

    Ok(())
}
```

**Result:**

```
deka-app file:
┌─────────────────────────────────────┐
│ 0x00000000: [Deka Binary]           │ ← OS loads this
│              180,000,000 bytes       │
│              (V8 + runtime + all)    │
├─────────────────────────────────────┤
│ 0x0AB61800: [VFS Data]              │ ← OS IGNORES this!
│              1,077 bytes             │
│              (compressed user code)  │
├─────────────────────────────────────┤
│ 0x0AB61C35: [Metadata Footer]       │ ← OS IGNORES this!
│              40 bytes                │
│              - Magic: "DEKAVFS1"    │
│              - VFS offset: 0x0AB61800│
│              - VFS size: 1077       │
│              - Entry point: handler.js│
└─────────────────────────────────────┘
```

## How We Read It Back

When `deka-app` runs, it reads its own file:

```rust
// In vfs_loader.rs
pub fn detect_embedded_vfs() -> Option<VfsProvider> {
    // 1. Get our own binary path
    let exe_path = std::env::current_exe()?;  // /tmp/deka-app

    // 2. Extract VFS using the binary.rs function
    let (vfs_bytes, metadata) = extract_vfs(&exe_path)?;

    // 3. Deserialize VFS
    let vfs = VFS::from_bytes(&vfs_bytes)?;

    Some(VfsProvider::new(vfs))
}

// In binary.rs
pub fn extract_vfs(binary_path: &Path) -> Result<(Vec<u8>, BinaryMetadata), String> {
    // 1. Open the binary as a file
    let mut file = File::open(binary_path)?;

    // 2. Read entire file into memory
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;  // Read all 180,001,077 bytes

    // 3. Search backwards for magic bytes "DEKAVFS1"
    let min_metadata_size = 28;
    for i in (0..contents.len().saturating_sub(min_metadata_size)).rev() {
        if &contents[i..i + 8] == VFS_MAGIC {
            // 4. Found it! Parse metadata
            let metadata_bytes = &contents[i..];
            if let Ok(metadata) = BinaryMetadata::from_bytes(metadata_bytes) {
                // 5. Extract VFS data using offset from metadata
                let vfs_start = metadata.vfs_offset as usize;
                let vfs_end = vfs_start + metadata.vfs_size as usize;

                if vfs_end <= contents.len() {
                    let vfs_data = contents[vfs_start..vfs_end].to_vec();
                    return Ok((vfs_data, metadata));
                }
            }
        }
    }

    Err("No VFS data found in binary".to_string())
}
```

## ELF/Mach-O Binary Structure

Actual `deka-app` file (hex dump):

```
┌─────────────────────────────────────────────────┐
│ Offset    │ Bytes                               │
├─────────────────────────────────────────────────┤
│ 0x000000  │ 7F 45 4C 46 02 01 01 00 ...        │ ← ELF Magic
│ 0x000040  │ [Program Headers]                  │
│           │   LOAD segment: offset=0x100       │ ← OS reads this
│           │              filesz=0x5000         │
│           │   LOAD segment: offset=0x5000      │
│           │              filesz=0xA000         │
│ 0x000100  │ [.text section - executable code]  │
│ 0x005000  │ [.data section - initialized data] │
│ 0x00F000  │ [.rodata - read-only data]         │
│ 0x0AB61800│ [END OF DECLARED SECTIONS] ←─┐     │
│           │                               │     │
│ 0x0AB61800│ 1F 8B 08 00 ... [VFS Data]   │ OS  │
│           │                               │ never│
│ 0x0AB61C35│ 44 45 4B 41 56 46 53 31      │ reads│
│           │ "DEKAVFS1" [Metadata]         │ this!│
└─────────────────────────────────────────────────┘
```

## OS Binary Loading (Simplified)

When you run `./deka-app`, the OS kernel does:

```c
// Simplified kernel code:
elf_header = read_bytes(file, 0, 64);
program_headers = read_bytes(file, elf_header.phoff, header_size);

for (segment in program_headers) {
    if (segment.type == PT_LOAD) {
        // Load this segment into memory
        memory = allocate(segment.memsz);
        read_bytes(file, segment.offset, segment.filesz);
        memcpy(memory, segment_data, segment.filesz);
        // Everything after segment.filesz is IGNORED!
    }
}

// Jump to entry point
start_program(elf_header.entry);
```

**The OS never knows about the VFS data!**

## Why Metadata Goes at the End

We put metadata **at the very end** to avoid changing any offsets:

```
❌ BAD: Metadata at the beginning
┌─────────────────────────────────┐
│ [Metadata - 40 bytes]           │ ← Changes offset of EVERYTHING
│ [Binary]                        │ ← Code now at wrong address!
│ [VFS]                           │
└─────────────────────────────────┘
BREAKS: ELF header says code at 0x100, but now it's at 0x128!

✅ GOOD: Metadata at the end
┌─────────────────────────────────┐
│ [Binary]                        │ ← Code still at correct address
│ [VFS]                           │
│ [Metadata - 40 bytes]           │ ← Doesn't affect anything!
└─────────────────────────────────┘
WORKS: ELF header unchanged, code at 0x100 ✓
```

## The Magic Bytes Trick

```rust
const VFS_MAGIC: &[u8; 8] = b"DEKAVFS1";
```

**Why 8 bytes?**
- Fast to compare on 64-bit CPUs (single instruction!)
- Unlikely to appear randomly in binary data
- Easy to search for
- Fits in a CPU register

**Search algorithm:**

```rust
// Search from the end backwards (metadata is at the end)
for i in (0..file_len).rev() {
    if bytes[i..i+8] == b"DEKAVFS1" {
        // Found it! This is our metadata!
        parse_metadata(&bytes[i..])
    }
}
```

## Metadata Format

The metadata footer has a variable size but predictable structure:

```
Metadata Footer:
┌────────────────────────────────────┐
│ Offset │ Size │ Field              │
├────────────────────────────────────┤
│ +0     │ 8    │ Magic: "DEKAVFS1"  │
│ +8     │ 8    │ VFS offset (u64)   │
│ +16    │ 8    │ VFS size (u64)     │
│ +24    │ 4    │ Entry point len    │
│ +28    │ N    │ Entry point string │
└────────────────────────────────────┘
Total size: 28 + len(entry_point) bytes
```

**Example for "handler.js":**

```
Hex dump:
0x00: 44 45 4B 41 56 46 53 31    "DEKAVFS1"
0x08: 00 18 B6 0A 00 00 00 00    offset = 180,000,000 (little-endian)
0x10: 35 04 00 00 00 00 00 00    size = 1,077 (little-endian)
0x18: 0A 00 00 00                len = 10 (little-endian)
0x1C: 68 61 6E 64 6C 65 72 2E    "handler.js"
      6A 73

Total: 38 bytes
```

## Serialization/Deserialization

```rust
// Serialize (to_bytes)
impl BinaryMetadata {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // 1. Magic (8 bytes)
        bytes.extend_from_slice(&self.magic);

        // 2. VFS offset (8 bytes, little-endian)
        bytes.extend_from_slice(&self.vfs_offset.to_le_bytes());

        // 3. VFS size (8 bytes, little-endian)
        bytes.extend_from_slice(&self.vfs_size.to_le_bytes());

        // 4. Entry point length (4 bytes, little-endian)
        let entry_len = self.entry_point.len() as u32;
        bytes.extend_from_slice(&entry_len.to_le_bytes());

        // 5. Entry point string (N bytes)
        bytes.extend_from_slice(self.entry_point.as_bytes());

        bytes
    }
}

// Deserialize (from_bytes)
impl BinaryMetadata {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        // 1. Verify minimum size
        if bytes.len() < 28 {
            return Err("Metadata too short".to_string());
        }

        // 2. Read magic
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&bytes[0..8]);
        if &magic != VFS_MAGIC {
            return Err("Invalid magic bytes".to_string());
        }

        // 3. Read VFS offset
        let vfs_offset = u64::from_le_bytes(bytes[8..16].try_into().unwrap());

        // 4. Read VFS size
        let vfs_size = u64::from_le_bytes(bytes[16..24].try_into().unwrap());

        // 5. Read entry point length
        let entry_len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;

        // 6. Read entry point string
        if bytes.len() < 28 + entry_len {
            return Err("Metadata truncated".to_string());
        }
        let entry_point = String::from_utf8(bytes[28..28 + entry_len].to_vec())?;

        Ok(Self { magic, vfs_offset, vfs_size, entry_point })
    }
}
```

## Real-World Example

You can test this yourself:

```bash
# Create a simple binary
$ echo 'int main() { return 0; }' > test.c
$ gcc test.c -o test
$ ls -l test
-rwxr-xr-x  1 user  staff  16384 Jan 10 02:00 test

# Look at the end
$ xxd test | tail -n 2
00003ff0: 0000 0000 0000 0000 0000 0000 0000 0000  ................
00004000:                                          # ← File ends here

# Append data to it!
$ echo "Hello from appended data!" >> test
$ ls -l test
-rwxr-xr-x  1 user  staff  16410 Jan 10 02:00 test  # ← 26 bytes bigger!

# The binary STILL WORKS!
$ ./test
$ echo $?
0  # ← Exits successfully! OS doesn't care about extra bytes.

# But we can read the appended data:
$ tail -c 26 test
Hello from appended data!
```

## Performance Considerations

**Question:** Reading the whole file into memory seems slow?

**Answer:** We only do it **once** at startup:

```rust
// Startup (once, ~100ms):
let vfs = detect_embedded_vfs()?;  // Reads file into memory
mount_vfs(vfs);                    // Extracts to HashMap cache

// During execution (thousands of times, <1μs each):
let content = read_from_vfs("handler.js")?;  // HashMap lookup in RAM
```

**Optimization:** Could use `mmap` to memory-map the file instead of reading it all:

```rust
// Future optimization:
let mmap = unsafe { MmapOptions::new().map(&file)? };
let vfs_data = &mmap[vfs_offset..vfs_offset + vfs_size];
```

## Cross-Platform Considerations

### Same-Platform Compilation (Current)

```
macOS:
  deka (macOS binary) → deka-app (macOS binary)

Linux:
  deka (Linux binary) → deka-app (Linux binary)
```

We copy the **currently running binary**, so output matches the host platform.

### Cross-Compilation (Future)

To support cross-compilation (`deka compile --target linux-x64` on macOS):

**Option 1: Bundle pre-built binaries**
```
deka/templates/
├── deka-darwin-arm64    (180MB)
├── deka-darwin-x64      (180MB)
├── deka-linux-x64       (185MB)
├── deka-linux-arm64     (178MB)
├── deka-windows-x64.exe (190MB)
```

Download size: ~1GB

**Option 2: On-demand download**
```rust
fn get_template(target: &str) -> PathBuf {
    let cache = "~/.deka/templates/";
    let template = format!("{}/deka-{}", cache, target);

    if !exists(&template) {
        download(
            format!("https://releases.deka.sh/{}/deka", target),
            &template
        );
    }

    template
}
```

Download size: 180MB per platform (on first use)

## Security Considerations

### Read-Only VFS

The VFS is **read-only** - user code cannot modify the embedded files:

```rust
impl VfsProvider {
    pub fn read_file(&mut self, path: &str) -> Result<String, String> {
        // ✅ Reading is allowed
    }

    // ❌ No write_file() method - VFS is immutable
}
```

### Code Signature Impact

Appending data **invalidates code signatures**:

```bash
# Original binary (signed)
$ codesign -v deka
deka: valid on disk

# After appending VFS
$ codesign -v deka-app
deka-app: code object is not signed at all
```

**Solution:** Re-sign after embedding:
```bash
$ codesign -s "Developer ID" deka-app
```

This is expected and how all bundlers work.

## Related Work

This technique is used by many projects:

1. **Bun** - JavaScript/TypeScript bundler
   - Uses same append technique
   - Prefix: `/$bunfs/` for VFS paths

2. **Deno** - TypeScript runtime
   - `deno compile` uses similar approach
   - Bundles with `deno_standalone`

3. **PyInstaller** - Python bundler
   - Appends Python bytecode to binary
   - Uses CArchive format

4. **UPX** - Executable packer
   - Appends compressed binary
   - Decompresses at runtime

5. **Self-extracting archives** - WinZip, 7-Zip
   - Executable + compressed data
   - Extracts to temp on run

## References

- **ELF Format:** [ELF Specification](https://refspecs.linuxfoundation.org/elf/elf.pdf)
- **Mach-O Format:** [macOS ABI](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/CodeFootprint/Articles/MachOOverview.html)
- **PE Format (Windows):** [PE Format Specification](https://docs.microsoft.com/en-us/windows/win32/debug/pe-format)
- **Bun Source:** [StandaloneModuleGraph.zig](https://github.com/oven-sh/bun/blob/main/src/StandaloneModuleGraph.zig)

## Summary

**The key insight:**

1. Executables are just **files with special headers**
2. The OS only reads what the headers **declare**
3. We can append **arbitrary data** after declared sections
4. Our program can **read its own file** to access that data
5. We use **magic bytes** to find the appended data
6. No compilation needed - just **copying bytes**!

This is why `deka compile` is:
- ⚡ **Fast** (milliseconds, not minutes)
- 📦 **Zero dependencies** (no compilers needed)
- 🔧 **Simple** (just file I/O)
- 🎯 **Portable** (works on any platform)

It's a decades-old trick that still feels like magic! ✨
