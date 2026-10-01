//! Shared native executable signing layout, independent of either runtime.
#[cfg(target_os = "macos")]
use std::{fs::File, io::Read};

/// Native Darwin binaries use the little-endian 64-bit Mach-O layout.
/// Include the payload in __LINKEDIT so codesign preserves and signs it.
#[cfg(target_os = "macos")]
pub fn extend_linkedit(bytes: &mut [u8], end: u64) -> Result<(), String> {
    let commands = macho_commands(bytes)?;
    for offset in commands {
        if u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) == 0x19
            && bytes.get(offset + 8..offset + 24) == Some(b"__LINKEDIT\0\0\0\0\0\0")
        {
            if offset + 72 > bytes.len() {
                return Err("truncated __LINKEDIT".into());
            }
            let start = u64::from_le_bytes(bytes[offset + 40..offset + 48].try_into().unwrap());
            let size = end.checked_sub(start).ok_or("invalid __LINKEDIT offset")?;
            bytes[offset + 48..offset + 56].copy_from_slice(&size.to_le_bytes());
            bytes[offset + 32..offset + 40]
                .copy_from_slice(&((size + 16383) & !16383).to_le_bytes());
            return Ok(());
        }
    }
    Err("Mach-O executable has no __LINKEDIT segment".into())
}

#[cfg(target_os = "macos")]
fn macho_commands(bytes: &[u8]) -> Result<Vec<usize>, String> {
    if bytes.len() < 32 || bytes[..4] != [0xcf, 0xfa, 0xed, 0xfe] {
        return Err("expected a native 64-bit Mach-O executable".into());
    }
    let count = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let mut offset = 32;
    let mut commands = Vec::new();
    for _ in 0..count {
        if offset + 8 > bytes.len() {
            return Err("truncated Mach-O command".into());
        }
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if size < 8 || size > bytes.len() - offset {
            return Err("invalid Mach-O command size".into());
        }
        commands.push(offset);
        offset += size;
    }
    Ok(commands)
}

/// codesign appends a potentially large signature after the payload footer.
/// Return the end of signed data so the runtime can retain its bounded scan.
#[cfg(target_os = "macos")]
pub fn signed_data_end(file: &mut File) -> Result<Option<u64>, String> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut header = [0; 32];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header[..4] != [0xcf, 0xfa, 0xed, 0xfe] {
        return Ok(None);
    }
    let size = u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
    if size > 1024 * 1024 {
        return Err("Mach-O load commands too large".into());
    }
    let mut bytes = vec![0; 32 + size];
    bytes[..32].copy_from_slice(&header);
    file.read_exact(&mut bytes[32..])
        .map_err(|e| e.to_string())?;
    for offset in macho_commands(&bytes)? {
        if u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) == 0x1d {
            let data = bytes
                .get(offset + 8..offset + 16)
                .ok_or("truncated code signature command")?;
            let start = u32::from_le_bytes(data[..4].try_into().unwrap());
            if start != 0 {
                return Ok(Some(start as u64));
            }
        }
    }
    Ok(None)
}
