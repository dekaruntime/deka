//! The testsuite runner's package cache must key on resolved versions, not
//! package names (dekaruntime/deka#535). A name-only key served pre-release
//! package bytes to a post-release compiler; the failure blamed the package
//! source and read as a compiler bug, and it defeated --locked because the
//! cache was consulted before the lock was honoured.
//!
//! This test replays that scenario hermetically: a committed cache entry
//! holds the REAL @deka/io@0.1.1 install (captured from a live install; its
//! lock's integrity hashes match by construction), and a poisoned entry sits
//! under the OLD name-only key. The runner must hit the versioned entry —
//! never the poisoned one — so no network is needed: a cache hit never runs
//! `deka add`. On the old name-only behavior the poison is served, the
//! fixture fails to compile, and this test fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("package-cache")
}

fn release_cli(root: &Path) -> PathBuf {
    let cli = root.join("target").join("release").join("cli");
    assert!(
        cli.exists(),
        "testsuite runner needs target/release/cli; run `cargo build --release -p cli` first"
    );
    cli
}

fn bun() -> String {
    let output = Command::new("which")
        .arg("bun")
        .output()
        .expect("spawn which");
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!path.is_empty(), "bun is required on PATH to run run.mjs");
    path
}

/// Copy the committed cache entry into the shared cache, alongside a poisoned
/// entry under the old name-only key. Returns the two cache dirs for cleanup.
fn seed_cache(root: &Path) -> (PathBuf, PathBuf) {
    let cache_root = root.join(".cache").join("deka-packages");
    let versioned = cache_root.join("@deka").join("io@0.1.1");
    let name_only = cache_root.join("io");

    if versioned.exists() {
        fs::remove_dir_all(&versioned).expect("clear versioned cache dir");
    }
    fs::create_dir_all(&versioned).expect("versioned cache dir");
    copy_dir_all(&fixtures().join("cache-entry"), &versioned);

    if name_only.exists() {
        fs::remove_dir_all(&name_only).expect("clear poison cache dir");
    }
    let poison_pkg = name_only
        .join("ds_modules")
        .join("@deka")
        .join("io");
    fs::create_dir_all(&poison_pkg).expect("poison pkg dir");
    // Serves bytes that fail to compile: if the runner consults the old
    // name-only key, the fixture breaks and the test fails.
    fs::write(
        poison_pkg.join("index.ds"),
        "const broken: number = \"poison\"\n",
    )
    .expect("poison source");
    fs::write(name_only.join("deka.lock"), "{}\n").expect("poison lock");

    (versioned, name_only)
}

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("copy dst");
    for entry in fs::read_dir(src).expect("copy src") {
        let entry = entry.expect("entry");
        let to = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir_all(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).expect("copy file");
        }
    }
}

#[test]
fn package_cache_key_uses_resolved_versions_not_names() {
    let root = repo_root();
    let (versioned, name_only) = seed_cache(&root);

    let output = Command::new(bun())
        .arg(root.join("scripts").join("testsuite-run.mjs"))
        .arg("--root")
        .arg(fixtures().join("corpus"))
        .arg("--filter")
        .arg("zcache")
        .arg("--verbose")
        .env("DEKA_NATIVE", release_cli(&root))
        .output()
        .expect("spawn run.mjs");

    // Best-effort cleanup; leftover dirs are inert (the versioned key is
    // content-addressed and the name-only key is never consulted).
    let _ = fs::remove_dir_all(&versioned);
    let _ = fs::remove_dir_all(&name_only);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "fixture must pass using the cached @deka/io@0.1.1 entry\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("[deka-packages cache hit] @deka/io@0.1.1"),
        "runner must log the versioned cache hit; a silent hit is what made \
         deka#535 slow to find.\nstderr: {stderr}"
    );
}
