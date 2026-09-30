use serde::Deserialize;
use std::{collections::HashMap, error::Error, fs, path::PathBuf, process::Command};
use tauri_bundler::{BundleBinary, BundleSettings, PackageSettings, PackageType, SettingsBuilder};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Project {
    name: String,
    version: String,
    desktop: Desktop,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Desktop {
    product_name: String,
    identifier: String,
    entry: String,
    entry_function: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icons: Vec<String>,
    #[serde(default)]
    resources: HashMap<String, String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("dvm-package: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("this packaging proof of concept currently targets macOS .app bundles".into());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !matches!(args.len(), 4 | 6)
        || args[2] != "--out"
        || (args.len() == 6 && args[4] != "--bytecode")
    {
        return Err(
            "usage: dvm-package path/deka.json path/to/runtime --out output-directory [--bytecode app.dvm.json]".into(),
        );
    }
    let manifest = fs::canonicalize(&args[0])?;
    let project_dir = manifest
        .parent()
        .ok_or("manifest has no parent directory")?;
    let project: Project = serde_json::from_slice(&fs::read(&manifest)?)?;
    for (label, value) in [
        ("name", &project.name),
        ("desktop.productName", &project.desktop.product_name),
        ("desktop.identifier", &project.desktop.identifier),
    ] {
        if value.is_empty() || value.contains(['/', '\\']) || value == "." || value == ".." {
            return Err(format!("invalid {label}").into());
        }
    }
    let program = if args.len() == 6 {
        let program: deka_vm::Program = serde_json::from_slice(&fs::read(&args[5])?)?;
        program.validate()?;
        program
    } else {
        let source = fs::read_to_string(project_dir.join(&project.desktop.entry))?;
        deka_vm::compiler::compile_entry(
            &source,
            &deka_vm::Hosts::default(),
            &project.desktop.entry_function,
        )?
    };
    let runtime = fs::canonicalize(&args[1])?;
    let out = PathBuf::from(&args[3]);
    fs::create_dir_all(&out)?;
    let out = fs::canonicalize(out)?;
    let payload = out.join("app.dvm.json");
    fs::write(&payload, serde_json::to_vec(&program)?)?;
    let executable_name = "deka-app";
    let executable = out.join(executable_name);
    if runtime == executable {
        return Err("runtime must be outside the packaging staging directory".into());
    }
    fs::copy(runtime, &executable)?;
    let mut resources = HashMap::new();
    for (source, destination) in project.desktop.resources {
        let path = std::path::Path::new(&destination);
        if destination.is_empty()
            || !path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            || destination == "app.dvm.json"
        {
            return Err(format!("invalid or reserved resource destination: {destination}").into());
        }
        let source = fs::canonicalize(project_dir.join(source))?;
        resources.insert(source.to_string_lossy().into_owned(), destination);
    }
    resources.insert(
        payload.to_string_lossy().into_owned(),
        "app.dvm.json".into(),
    );
    let icons = project
        .desktop
        .icons
        .into_iter()
        .map(|icon| {
            fs::canonicalize(project_dir.join(icon)).map(|p| p.to_string_lossy().into_owned())
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let settings = SettingsBuilder::new()
        .project_out_directory(&out)
        .package_types(vec![PackageType::MacOsBundle])
        .package_settings(PackageSettings {
            product_name: project.desktop.product_name,
            version: project.version,
            description: project.desktop.description,
            homepage: None,
            authors: None,
            default_run: Some(executable_name.into()),
        })
        .bundle_settings(BundleSettings {
            identifier: Some(project.desktop.identifier),
            icon: Some(icons),
            resources_map: Some(resources),
            ..Default::default()
        })
        .binaries(vec![BundleBinary::new(executable_name.into(), true)])
        // This local demo never reads signing credentials or contacts Apple's service.
        .no_sign(true)
        .build()?;
    for bundle in tauri_bundler::bundle_project(&settings)? {
        for path in bundle.bundle_paths {
            // Local ad-hoc signature: no identity/keychain or distribution credentials.
            let status = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&path)
                .status()?;
            if !status.success() {
                return Err("local ad-hoc signing failed".into());
            }
            println!("{}", path.display());
        }
    }
    Ok(())
}
