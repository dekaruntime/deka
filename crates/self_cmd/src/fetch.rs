//! `deka self fetch <target>` — download a pinned content checkout (RFD 59,
//! deka#836).
//!
//! Downloads the archive for the target's embedded pin, hard-fails on a
//! SHA-256 mismatch (a moved tag or an altered response is an error, never a
//! silent corpus swap), and extracts the repository into `./<name>` in the
//! working directory. Replaces `scripts/ci-fetch-testsuite-corpus.sh` and
//! `scripts/ci-fetch-tour.sh`; the pin files those scripts read are now
//! embedded here at build time, so the pin keeps a single owner.
//!
//! Output follows RFD 55: `[downloading] testsuite` on stderr, then a
//! completion line naming the resolved ref and destination.

use super::targets::{self, ContentTarget};
use core::Context;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use stdio;

pub fn cmd(context: &Context) {
    let result = match context.args.positionals.get(0).map(|s| s.as_str()) {
        Some(name) => match targets::by_name(name) {
            Some(target) if name == target.name => fetch(target, &context.env.cwd),
            // The `suite` alias is accepted by `self test`, not `self fetch`.
            Some(_) => Err(format!("unknown fetch target '{}'", name)),
            None => Err(format!("unknown fetch target '{}'", name)),
        },
        None => Err(format!(
            "missing target name ({})",
            targets::fetch_names().join(", ")
        )),
    };

    if let Err(message) = result {
        stdio::error("self fetch", &message);
        std::process::exit(1);
    }
}

/// Download, verify, and extract `target` into `./<name>` under `cwd`.
/// Returns the checkout path.
pub fn fetch(target: &ContentTarget, cwd: &Path) -> Result<PathBuf, String> {
    let pin = targets::Pin::parse(target)?;
    let url = targets::archive_url(target, &pin.reference);

    stdio::log("downloading", target.name);

    let temp = tempfile::tempdir()
        .map_err(|err| format!("failed to create a temp directory: {}", err))?;
    let archive = temp.path().join(format!("{}.tar.gz", target.name));
    download(&url, &archive)?;
    verify_sha256(&archive, &pin.sha256).map_err(|message| {
        format!(
            "{} checksum guard: {} (expected pin from scripts/{}-version)",
            target.name, message, target.name
        )
    })?;

    let dest = cwd.join(target.name);
    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|err| format!("failed to replace {}: {}", dest.display(), err))?;
    }
    extract_archive(&archive, &dest)?;

    let marker = dest.join(target.marker_file);
    if !marker.is_file() {
        let _ = fs::remove_dir_all(&dest);
        return Err(format!(
            "{} archive does not contain {}; refusing a partial checkout",
            target.name, target.marker_file
        ));
    }

    stdio::log(
        "self fetch",
        &format!("{} {} -> {}", target.name, pin.reference, dest.display()),
    );
    Ok(dest)
}

/// Download `url` to `dest`, failing on any non-success status.
fn download(url: &str, dest: &Path) -> Result<(), String> {
    let mut response = reqwest::blocking::get(url)
        .map_err(|err| format!("download failed for {}: {}", url, err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("download failed for {}: HTTP {}", url, status));
    }
    let mut file = fs::File::create(dest)
        .map_err(|err| format!("failed to create {}: {}", dest.display(), err))?;
    response
        .copy_to(&mut file)
        .map_err(|err| format!("failed writing {}: {}", dest.display(), err))?;
    Ok(())
}

/// Verify `path` against a lowercase SHA-256 hex digest. A mismatch is a
/// hard error — this is the checksum guard the fetch scripts carried.
fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let mut file =
        fs::File::open(path).map_err(|err| format!("failed to open archive: {}", err))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|err| format!("failed to hash archive: {}", err))?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(format!("checksum mismatch: expected {}, got {}", expected, actual));
    }
    Ok(())
}

/// Extract a GitHub `.tar.gz` archive into `dest`, stripping the single
/// top-level `<repo>-<ref>/` wrapper directory. Paths are sanitized: any
/// entry that would escape `dest` is rejected. Modes are normalized to a
/// private checkout (see [`normalize_mode`]) — the archive's own
/// group-writable modes are a GitHub packaging artifact, not a permission
/// the fetched content asked for.
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let file =
        fs::File::open(archive).map_err(|err| format!("failed to open archive: {}", err))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    for entry in tar
        .entries()
        .map_err(|err| format!("failed to read archive: {}", err))?
    {
        let mut entry =
            entry.map_err(|err| format!("failed to read archive entry: {}", err))?;
        let path = entry
            .path()
            .map_err(|err| format!("corrupt archive path: {}", err))?;
        // Strip the archive's single wrapper directory.
        let rel = match path.components().count() {
            0 | 1 => continue,
            _ => path
                .components()
                .skip(1)
                .collect::<std::path::PathBuf>(),
        };
        // Refuse anything that is not a plain relative path inside dest.
        if rel
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!("archive entry escapes the checkout: {}", path.display()));
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
        }
        entry
            .unpack(&out)
            .map_err(|err| format!("failed to extract {}: {}", rel.display(), err))?;
        normalize_mode(&out, entry.header())
            .map_err(|err| format!("failed to set permissions on {}: {}", rel.display(), err))?;
    }
    Ok(())
}

/// Normalize an extracted entry's mode to a private, single-user checkout.
///
/// GitHub serves archives whose directories are group-writable (0775) and
/// whose files are group-writable too (0664); faithfully preserving those
/// modes makes every content runner that enforces a secure output topology
/// (all ancestors of its output dirs owned by the effective user and free of
/// group/other write — see dekaruntime/tour `secureOutputTopologyError`)
/// refuse the freshly fetched checkout as "environment unfit to run". The
/// extractor owns the checkout layout, so it lays it out private: directories
/// 0755, files 0644 plus whatever exec bit the archive carried (the pinned
/// toolchains carry real executables). The pin is over archive bytes, never
/// modes — normalizing here cannot weaken the checksum guard.
#[cfg(unix)]
fn normalize_mode(out: &Path, header: &tar::Header) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = match header.entry_type() {
        tar::EntryType::Directory => 0o755,
        tar::EntryType::Regular => 0o644 | (header.mode()? & 0o111),
        _ => return Ok(()),
    };
    fs::set_permissions(out, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn normalize_mode(_out: &Path, _header: &tar::Header) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn sha256_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Build a GitHub-style `.tar.gz` in memory: one wrapper directory
    /// containing `files` (relative path -> contents).
    fn build_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            for (rel, contents) in files {
                let path = format!("wrapper-1.0.0/{}", rel);
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, &path, *contents)
                    .expect("append fixture file");
            }
            tar.finish().expect("finish tar");
        }
        gz.finish().expect("finish gzip")
    }

    #[test]
    fn sha256_guard_accepts_matching_archive() {
        let bytes = b"trusted corpus bytes";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("archive.tar.gz");
        fs::write(&file, bytes).unwrap();
        assert!(verify_sha256(&file, &sha256_of(bytes)).is_ok());
    }

    #[test]
    fn tampered_archive_fails_the_checksum_guard() {
        let bytes = b"trusted corpus bytes";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("archive.tar.gz");
        fs::write(&file, bytes).unwrap();
        let expected = sha256_of(b"different bytes than what was pinned");
        let err = verify_sha256(&file, &expected).expect_err("tampered archive must fail");
        assert!(err.contains("checksum mismatch"), "unexpected error: {}", err);
    }

    #[test]
    fn extraction_strips_the_wrapper_directory() {
        let archive = build_archive(&[
            ("corpus/expected-failures.txt", b"# none\n"),
            ("corpus/basics/ok.pass.ds", b"export const ok = 1\n"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("archive.tar.gz");
        fs::write(&tar_path, &archive).unwrap();
        let dest = dir.path().join("testsuite");

        extract_archive(&tar_path, &dest).expect("extract");

        assert_eq!(
            fs::read_to_string(dest.join("corpus/expected-failures.txt")).unwrap(),
            "# none\n"
        );
        assert!(dest.join("corpus/basics/ok.pass.ds").is_file());
        // The wrapper directory must not survive into the checkout.
        assert!(!dest.join("wrapper-1.0.0").exists());
    }

    #[test]
    fn extraction_normalizes_modes_to_a_private_checkout() {
        // GitHub archives carry group-writable dirs (0775) and files (0664).
        // A fetched checkout must land private to the effective user or the
        // content runners' secure-output-topology check refuses it (deka#845).
        let archive = build_archive_mixed_modes();
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("archive.tar.gz");
        fs::write(&tar_path, &archive).unwrap();
        let dest = dir.path().join("tour");

        extract_archive(&tar_path, &dest).expect("extract");

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dest.join("tests/tour"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "extracted directories must not be group-writable"
        );
        assert_eq!(
            fs::metadata(dest.join("tests/tour/manifest.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644,
            "extracted files must not be group-writable"
        );
        // The exec bit the archive recorded survives normalization (pinned
        // toolchains carry real executables).
        assert_eq!(
            fs::metadata(dest.join("tests/tour/run.mjs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    /// A GitHub-style archive with the modes codeload actually serves:
    /// directories 0775, files 0664, executables 0775.
    fn build_archive_mixed_modes() -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            // Directory entries: size 0, directory type.
            for rel in ["tests", "tests/tour"] {
                let path = format!("wrapper-1.0.0/{}", rel);
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o775);
                header.set_cksum();
                tar.append_data(&mut header, &path, std::io::empty())
                    .expect("append fixture directory");
            }
            for (rel, contents, mode) in [
                ("tests/tour/manifest.json", &b"[]\n"[..], 0o664),
                ("tests/tour/run.mjs", &b"#!/usr/bin/env bun\n"[..], 0o775),
            ] {
                let path = format!("wrapper-1.0.0/{}", rel);
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                tar.append_data(&mut header, &path, contents)
                    .expect("append fixture entry");
            }
            tar.finish().expect("finish tar");
        }
        gz.finish().expect("finish gzip")
    }

    #[test]
    fn extraction_rejects_path_escape() {
        // Hand-built tar: the `tar` crate refuses to *write* a traversal
        // path, but a hostile server can serve one, so the extractor must
        // reject it on read.
        let archive = raw_tar_entry("wrapper-1.0.0/../evil", b"escape");
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("archive.tar.gz");
        fs::write(&tar_path, &archive).unwrap();

        let err = extract_archive(&tar_path, dir.path().join("out").as_path())
            .expect_err("traversal entry must be rejected");
        assert!(err.contains("escapes the checkout"), "unexpected error: {}", err);
        assert!(!dir.path().join("evil").exists());
    }

    /// A gzip'd single-entry tar with an unnormalized `name`, exactly as a
    /// hostile archive would carry it.
    fn raw_tar_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let octal = |field: &mut [u8], value: u64| {
            let text = format!("{:0width$o}", value, width = field.len() - 1);
            field[..text.len()].copy_from_slice(text.as_bytes());
        };
        octal(&mut header[100..108], 0o644); // mode
        octal(&mut header[108..116], 0); // uid
        octal(&mut header[116..124], 0); // gid
        octal(&mut header[124..136], contents.len() as u64); // size
        octal(&mut header[136..148], 0); // mtime
        header[156] = b'0'; // regular file
        header[257..263].copy_from_slice(b"ustar\0");
        // Checksum: spaces while summing, then the total.
        for byte in &mut header[148..156] {
            *byte = b' ';
        }
        let sum: u64 = header.iter().map(|&byte| byte as u64).sum();
        let text = format!("{:06o}\0 ", sum);
        header[148..156].copy_from_slice(text.as_bytes());

        let mut tar = Vec::new();
        tar.extend_from_slice(&header);
        tar.extend_from_slice(contents);
        tar.resize(tar.len().next_multiple_of(512), 0);
        tar.resize(tar.len() + 1024, 0); // end-of-archive blocks

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn download_writes_response_body() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = b"archive-bytes";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("downloaded");
        download(&format!("http://{}/x.tar.gz", addr), &dest).expect("download");
        assert_eq!(fs::read(&dest).unwrap(), body);
        server.join().unwrap();
    }
}
