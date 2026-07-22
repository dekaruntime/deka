use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let runtime_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("runtime root");
    let stdlib_root = runtime_root.join("php_modules");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    let generated = out_dir.join("stdlib_snapshot.rs");

    println!("cargo:rerun-if-changed={}", stdlib_root.display());

    let mut files = Vec::new();
    collect_files(&stdlib_root, &stdlib_root, &mut files).expect("collect stdlib files");
    files.sort();

    let mut out = fs::File::create(&generated).expect("create stdlib snapshot");
    writeln!(out, "pub const STDLIB_FILES: &[(&str, &[u8])] = &[").expect("write header");
    for (relative, absolute) in files {
        println!("cargo:rerun-if-changed={}", absolute.display());
        writeln!(
            out,
            "    ({:?}, include_bytes!({:?}) as &[u8]),",
            relative,
            absolute.display().to_string()
        )
        .expect("write file entry");
    }
    writeln!(out, "];").expect("write footer");
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("path under root")
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((relative, path));
        }
    }
    Ok(())
}
