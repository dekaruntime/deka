//! Check the unpublished local tree through the ordinary installed-consumer path.
use crate::packages;
use deka_vm::{Result, compiler, package::Lock};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn read_manifest(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)
        .map_err(|e| format!("cannot read package deka.json {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("{}: invalid deka.json: {e}", path.display()))
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or("staged source has no parent")?)
        .map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
fn dependency_owner(relative: &Path) -> Result<std::path::PathBuf> {
    let mut parts = relative.components();
    let first = parts.next().ok_or("missing installed package name")?;
    let mut owner = std::path::PathBuf::from(first.as_os_str());
    if first.as_os_str().to_string_lossy().starts_with('@') {
        owner.push(
            parts
                .next()
                .ok_or("missing scoped package name")?
                .as_os_str(),
        );
    }
    Ok(owner)
}
pub(crate) fn check(directory: &Path) -> Result<()> {
    let directory = directory
        .canonicalize()
        .map_err(|e| format!("package directory {}: {e}", directory.display()))?;
    let mut manifest = read_manifest(&directory.join("deka.json"))?;
    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or("package deka.json must contain a non-empty name")?
        .to_owned();
    let version = manifest
        .get("version")
        .map(|value| value.as_str().ok_or("package version must be a string"))
        .transpose()?
        .unwrap_or("0.0.0")
        .to_owned();
    packages::parse_spec(&format!("{name}@{version}"))?;
    let entry = packages::package_entry(&directory, &manifest)?;
    let dependencies = packages::dependency_pins(&manifest, &name)?;
    let hosts = crate::hosts()?;
    let inputs = compiler::package_check_inputs(&entry, &name, &hosts)?;
    if inputs.exports.is_empty() {
        return Err(format!(
            "package entry {} exports nothing; there is no consumer surface to verify",
            entry.display()
        ));
    }
    let scratch =
        tempfile::tempdir().map_err(|e| format!("cannot create package-check consumer: {e}"))?;
    let root = scratch.path();
    let installed = root.join("ds_modules").join(&name);
    let mut mappings = Vec::new();
    for (source, text) in inputs.sources {
        let relative = source.strip_prefix(&directory).map_err(|_| {
            format!(
                "package source is outside its directory: {}",
                source.display()
            )
        })?;
        let staged = if let Ok(relative) = relative.strip_prefix("ds_modules") {
            let owner = dependency_owner(relative)?;
            let original_manifest = directory.join("ds_modules").join(&owner).join("deka.json");
            let staged_manifest = root.join("ds_modules").join(&owner).join("deka.json");
            write(
                &staged_manifest,
                &fs::read(&original_manifest)
                    .map_err(|e| format!("{}: {e}", original_manifest.display()))?,
            )?;
            root.join("ds_modules").join(relative)
        } else {
            installed.join(relative)
        };
        write(&staged, text.as_bytes())?;
        mappings.push((staged, source));
    }
    // An explicit map keeps the checked package's dependency scope separate
    // from the scratch consumer's local self-dependency.
    manifest["dependencies"] = json!(dependencies);
    write(
        &installed.join("deka.json"),
        &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
    )?;
    let consumer_manifest = json!({"entry": "consumer.ds", "dependencies": {&name: &version}});
    write(
        &root.join("deka.json"),
        &serde_json::to_vec(&consumer_manifest).map_err(|e| e.to_string())?,
    )?;
    let lock_path = directory.join("deka.lock");
    let mut lock: Lock = if lock_path.exists() {
        serde_json::from_slice(&fs::read(&lock_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: invalid deka.lock: {e}", lock_path.display()))?
    } else {
        Lock::default()
    };
    // Only the self-pin is local: retain all installed dependency/version checks.
    lock.packages.insert(
        name.clone(),
        (
            version,
            String::new(),
            json!({"dependencies": dependencies}),
            String::new(),
        ),
    );
    write(
        &root.join("deka.lock"),
        &serde_json::to_vec(&lock).map_err(|e| e.to_string())?,
    )?;
    let mut consumer = String::new();
    for (index, exported) in inputs.exports.iter().enumerate() {
        let type_prefix = if exported.type_only { "type " } else { "" };
        if exported.name == "default" && !exported.type_only {
            consumer.push_str(&format!(
                "import {type_prefix}__package_export_{index} from {name:?};\n"
            ));
        } else {
            let exported_name = &exported.name;
            consumer.push_str(&format!(
                "import {type_prefix}{{{exported_name} as __package_export_{index}}} from {name:?};\n"
            ));
        }
    }
    let consumer_path = root.join("consumer.ds");
    write(&consumer_path, consumer.as_bytes())?;
    compiler::compile_file(&consumer_path, &hosts, None).map_err(|mut error| {
        for (staged, original) in mappings {
            error = error.replace(
                &staged.display().to_string(),
                &original.display().to_string(),
            );
        }
        error.replace(&consumer_path.display().to_string(), "<package consumer>")
    })?;
    Ok(())
}
