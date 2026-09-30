use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn build(
    source: &Path,
    compiler: &Path,
    out: &Path,
    runtime_shaders: bool,
) -> Result<PathBuf, String> {
    if out.exists() {
        return Err(format!(
            "output already exists: {} (choose a fresh directory)",
            out.display()
        ));
    }
    let compiled = Command::new(compiler)
        .arg("rust")
        .arg(source)
        .output()
        .map_err(|e| e.to_string())?;
    if !compiled.status.success() {
        return Err(String::from_utf8_lossy(&compiled.stderr).into_owned());
    }
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../deka_native_ui")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let runtime = runtime.to_str().ok_or("runtime path must be UTF-8")?;
    fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    fs::write(out.join("src/main.rs"), compiled.stdout).map_err(|e| e.to_string())?;
    let features = if runtime_shaders {
        "runtime-shaders"
    } else {
        "gpu"
    };
    let manifest = format!(
        "[package]\nname = \"deka-native-app\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\ndeka_native_ui = {{ path = {}, features = [\"{features}\"] }}\n[profile.release]\nstrip = true\n",
        serde_json::to_string(runtime).map_err(|e| e.to_string())?
    );
    fs::write(out.join("Cargo.toml"), manifest).map_err(|e| e.to_string())?;
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(out.join("Cargo.toml"))
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(
            "generated application build failed; source retained in output directory".into(),
        );
    }
    Ok(out.join("Cargo.toml"))
}
