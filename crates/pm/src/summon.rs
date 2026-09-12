//! Import-once foreign JavaScript. Network access ends before the install transaction.
use crate::{
    install::{InstallTransaction, recover_install_transaction},
    lock,
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use deka_modules::module_spec::{resolve_summoned_js_module_file, summoned_js_package_name};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path},
    time::Duration,
};
use swc_common::{FileName, SourceMap, sync::Lrc};
use swc_ecma_ast::*;
use swc_ecma_parser::{Parser, StringInput, Syntax, lexer::Lexer};
use swc_ecma_visit::{Visit, VisitWith};

const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub struct Summoned {
    pub spec: String,
    pub warnings: Vec<String>,
}

/// The callback chooses a new name on collision; existing packages are never overwritten.
pub fn summon_at(
    project: &Path,
    source: &str,
    mut conflict: impl FnMut(&str) -> Result<String>,
) -> Result<Summoned> {
    let project = project.canonicalize()?;
    recover_install_transaction(&project)?;
    let manifest_path = project.join("deka.json");
    let mut manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).context("summon requires deka.json; run deka init first")?,
    )?;
    let deps = manifest
        .as_object_mut()
        .ok_or_else(|| anyhow!("deka.json must be an object"))?
        .entry("dependencies")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("deka.json dependencies must be an object"))?;
    let lock_path = project.join("deka.lock");
    // Never let the compatibility reader silently replace malformed input.
    if lock_path.exists() {
        serde_json::from_slice::<lock::DekaLock>(&fs::read(&lock_path)?)?;
    }
    let mut locked = lock::read_lockfile_at(&lock_path);
    if source.starts_with("jsr:") {
        bail!("jsr: sources are planned for the next summon stage; use a JavaScript URL");
    }
    if source == "infer" {
        bail!("deka summon infer is a separate stage; write the summon block yourself");
    }
    let url = reqwest::Url::parse(source.strip_prefix("url:").unwrap_or(source))
        .context("expected an http(s):// or file:// JavaScript or .tgz URL")?;
    let mut bytes = Vec::new();
    match url.scheme() {
        "file" => {
            fs::File::open(
                url.to_file_path()
                    .map_err(|_| anyhow!("invalid file URL"))?,
            )?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        }
        "http" | "https" => {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()?
                .get(url.clone())
                .send()?
                .error_for_status()?
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)?;
        }
        _ => bail!(
            "unsupported summon source; use url: with http(s):// or file:// (jsr: is next stage)"
        ),
    }
    if bytes.len() as u64 > MAX_BYTES {
        bail!("summon source exceeds 64 MiB");
    }
    let staging = tempfile::Builder::new()
        .prefix(".deka-summon-")
        .tempdir_in(&project)?;
    let unpack = staging.path().join("unpack");
    fs::create_dir(&unpack)?;
    let basename = url
        .path_segments()
        .and_then(|s| s.filter(|p| !p.is_empty()).last())
        .unwrap_or("module");
    let archive = basename.ends_with(".tgz") || basename.ends_with(".tar.gz");
    let package = if archive {
        unpack_archive(&bytes, &unpack)?;
        if unpack.join("package/package.json").is_file() {
            unpack.join("package")
        } else {
            unpack.clone()
        }
    } else {
        fs::write(unpack.join("index.mjs"), &bytes)?;
        unpack.clone()
    };
    let package_json = package.join("package.json");
    let metadata: Value = if package_json.exists() {
        serde_json::from_slice(&fs::read(&package_json)?)?
    } else {
        json!({})
    };
    let fallback = basename
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".tgz")
        .trim_end_matches(".mjs")
        .trim_end_matches(".js");
    let mut name = match metadata.get("name") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| anyhow!("package.json name must be a string"))?
            .to_string(),
        None => fallback
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect(),
    };
    loop {
        let spec = format!("@js/{name}");
        if summoned_js_package_name(&spec).is_none() {
            bail!(
                "invalid vendor name {name:?}; @js/ requires an unscoped name containing letters, digits, '-' or '_'"
            );
        }
        if deps.contains_key(&spec)
            || locked.packages.contains_key(&spec)
            || project
                .join("js_modules")
                .join(&name)
                .symlink_metadata()
                .is_ok()
        {
            let next = conflict(&name)?;
            if next == name {
                bail!("summon conflict: {spec} already exists; choose a different name");
            }
            name = next;
        } else {
            break;
        }
    }
    let spec = format!("@js/{name}");
    // Use the external routing table even for staging: one entry-point contract.
    let stage_project = staging.path().join("project");
    let vendor = stage_project.join("js_modules").join(&name);
    fs::create_dir_all(vendor.parent().unwrap())?;
    fs::rename(package, &vendor)?;
    let entry = resolve_summoned_js_module_file(&stage_project, &spec).map_err(|e| anyhow!(e))?;
    let warnings = minified_warning(&url, &fs::read_to_string(&entry)?);
    validate_graph(&project, &vendor, &entry)?;
    let files = file_hashes(&vendor)?;
    let integrity = format!("sha256-{}", STANDARD.encode(Sha256::digest(&bytes)));
    locked.packages.insert(
        spec.clone(),
        (
            spec.clone(),
            url.to_string(),
            json!({"source": "url", "integrity": integrity, "files": files}),
            integrity,
        ),
    );
    deps.insert(spec.clone(), json!(format!("url:{url}")));
    let result = (|| {
        let mut transaction = InstallTransaction::begin(&project, &lock_path)?;
        transaction.commit_package(&vendor, &project.join("js_modules").join(&name))?;
        lock::write_lockfile_at(&lock_path, &locked)?;
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest)? + "\n",
        )?;
        fs::File::open(&manifest_path)?.sync_all()?;
        transaction.finish()
    })();
    if let Err(error) = result {
        recover_install_transaction(&project).context("summon rollback failed")?;
        return Err(error);
    }
    Ok(Summoned { spec, warnings })
}

fn unpack_archive(bytes: &[u8], root: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut size = 0u64;
    for item in archive.entries()? {
        let mut item = item?;
        let path = item.path()?.into_owned();
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            bail!("unsafe archive path: {}", path.display());
        }
        if path
            .components()
            .any(|component| component.as_os_str() == ".deka-staged-package")
        {
            bail!(
                "archive contains reserved pm transaction marker: {}",
                path.display()
            );
        }
        let kind = item.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            bail!(
                "summon archives may contain only files and directories: {}",
                path.display()
            );
        }
        size = size
            .checked_add(item.size())
            .ok_or_else(|| anyhow!("archive too large"))?;
        if size > MAX_BYTES {
            bail!("expanded summon archive exceeds 64 MiB");
        }
        if !item.unpack_in(root)? {
            bail!("archive path escapes package");
        }
    }
    Ok(())
}

fn file_hashes(root: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
        for item in fs::read_dir(dir)? {
            let item = item?;
            let path = item.path();
            let kind = item.file_type()?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                // pm's transaction marker is not part of the package.
                if item.file_name() == ".deka-staged-package" {
                    continue;
                }
                out.insert(
                    path.strip_prefix(root)?
                        .to_str()
                        .ok_or_else(|| anyhow!("non-UTF8 package path"))?
                        .replace('\\', "/"),
                    format!(
                        "sha256-{}",
                        STANDARD.encode(Sha256::digest(fs::read(path)?))
                    ),
                );
            } else {
                bail!("summoned package contains a symlink or special file");
            }
        }
        Ok(())
    }
    let mut hashes = BTreeMap::new();
    walk(root, root, &mut hashes)?;
    Ok(hashes)
}

/// Ordinary install is offline for @js: preserve pins and reject changed/missing vendor bytes.
pub(crate) fn verify_locked_at(project: &Path) -> Result<BTreeMap<String, lock::LockEntry>> {
    let path = project.join("deka.json");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let manifest: Value = serde_json::from_slice(&fs::read(path)?)?;
    let locked = lock::read_lockfile_at(&project.join("deka.lock"));
    let mut result = BTreeMap::new();
    if let Some(deps) = manifest.get("dependencies").and_then(Value::as_object) {
        for (spec, source) in deps.iter().filter(|(name, _)| name.starts_with("@js/")) {
            let name = summoned_js_package_name(spec)
                .ok_or_else(|| anyhow!("invalid summoned specifier {spec}"))?;
            let entry = locked
                .packages
                .get(spec)
                .ok_or_else(|| anyhow!("{spec} is not locked; run deka summon <source>"))?;
            if source.as_str() != Some(format!("url:{}", entry.1).as_str()) {
                bail!("{spec} source differs from deka.lock");
            }
            resolve_summoned_js_module_file(project, spec).map_err(|e| anyhow!(e))?;
            let expected: BTreeMap<String, String> = serde_json::from_value(
                entry
                    .2
                    .get("files")
                    .cloned()
                    .ok_or_else(|| anyhow!("{spec} has no vendored integrity pin"))?,
            )?;
            if file_hashes(&project.join("js_modules").join(name))? != expected {
                bail!("{spec} vendored integrity mismatch; restore the vetted bytes");
            }
            result.insert(spec.clone(), entry.clone());
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Imports {
    specs: BTreeSet<String>,
    unknown: bool,
}
impl Visit for Imports {
    fn visit_import_decl(&mut self, n: &ImportDecl) {
        self.specs
            .insert(n.src.value.to_string_lossy().into_owned());
    }
    fn visit_named_export(&mut self, n: &NamedExport) {
        if let Some(src) = &n.src {
            self.specs.insert(src.value.to_string_lossy().into_owned());
        }
    }
    fn visit_export_all(&mut self, n: &ExportAll) {
        self.specs
            .insert(n.src.value.to_string_lossy().into_owned());
    }
    fn visit_call_expr(&mut self, n: &CallExpr) {
        if matches!(&n.callee, Callee::Import(_))
            || matches!(&n.callee, Callee::Expr(e) if matches!(e.as_ref(), Expr::Ident(i) if i.sym == *"require"))
        {
            if let Some(ExprOrSpread { spread: None, expr }) = n.args.first() {
                if let Expr::Lit(Lit::Str(s)) = expr.as_ref() {
                    self.specs.insert(s.value.to_string_lossy().into_owned());
                } else {
                    self.unknown = true;
                }
            } else {
                self.unknown = true;
            }
        }
        n.visit_children_with(self);
    }
}

fn validate_graph(project: &Path, vendor: &Path, entry: &Path) -> Result<()> {
    let root = vendor.canonicalize()?;
    let mut pending = vec![entry.to_path_buf()];
    let mut seen = BTreeSet::new();
    let mut missing = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(FileName::Real(path.clone()).into(), source);
        let mut parser = Parser::new_from(Lexer::new(
            Syntax::Es(Default::default()),
            Default::default(),
            StringInput::from(&*fm),
            None,
        ));
        let module = parser
            .parse_module()
            .map_err(|e| anyhow!("{}: invalid JavaScript: {:?}", path.display(), e.kind()))?;
        if !parser.take_errors().is_empty() {
            bail!("{}: invalid JavaScript", path.display());
        }
        let mut imports = Imports::default();
        module.visit_with(&mut imports);
        if imports.unknown {
            missing.insert(
                "<non-literal dynamic import/require: use explicit module specifiers>".to_string(),
            );
        }
        for spec in imports.specs {
            if spec.starts_with("./") || spec.starts_with("../") {
                if let Ok(file) = path.parent().unwrap().join(&spec).canonicalize() {
                    if file.starts_with(&root) && file.is_file() {
                        pending.push(file);
                        continue;
                    }
                }
            } else if summoned_js_package_name(&spec).is_some() {
                if verify_locked_at(project)?.contains_key(&spec) {
                    continue;
                }
            }
            missing.insert(spec);
        }
    }
    if !missing.is_empty() {
        bail!(
            "no transitive fetching: unresolved inner imports (summon each explicitly, then reference its @js/<name> in a vetted module):\n{}",
            missing
                .into_iter()
                .map(|s| format!("  {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(())
}

fn minified_warning(url: &reqwest::Url, source: &str) -> Vec<String> {
    if !url.path().contains(".min.") && !source.lines().any(|line| line.len() > 1000) {
        return Vec::new();
    }
    let mut warning =
        "minified distribution detected; prefer a readable ES module build for review".to_string();
    if matches!(url.host_str(), Some("cdn.jsdelivr.net" | "unpkg.com"))
        && url.path().contains(".min.js")
    {
        let mut suggested = url.clone();
        suggested.set_path(&url.path().replace(".min.js", ".module.js"));
        warning.push_str(&format!(
            "; possible module build URL (check availability): {suggested}"
        ));
    }
    vec![warning]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minified_cdn_hint_is_nonblocking_and_does_not_fetch() {
        let url = reqwest::Url::parse("https://cdn.jsdelivr.net/npm/three/build/three.min.js?v=1")
            .unwrap();
        let warnings = minified_warning(&url, "export function f() {}");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("three.module.js?v=1"));
        assert!(
            minified_warning(
                &reqwest::Url::parse("https://example.com/readable.mjs").unwrap(),
                "export function f() {}\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn shared_transaction_recovers_vendor_manifest_and_lock() {
        let project = tempfile::tempdir().unwrap();
        let manifest = project.path().join("deka.json");
        let lock_path = project.path().join("deka.lock");
        fs::write(&manifest, "{\"custom\": 42}\n").unwrap();
        fs::write(&lock_path, "{\"lockfileVersion\":1,\"packages\":{}}\n").unwrap();
        let old_manifest = fs::read(&manifest).unwrap();
        let old_lock = fs::read(&lock_path).unwrap();
        let stage = project.path().join("staging/package");
        let vendor = project.path().join("js_modules/example");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("index.mjs"), "export const x = 1;").unwrap();
        let mut transaction = InstallTransaction::begin(project.path(), &lock_path).unwrap();
        transaction.commit_package(&stage, &vendor).unwrap();
        fs::write(&manifest, "{}\n").unwrap();
        fs::write(&lock_path, "{}\n").unwrap();
        drop(transaction);
        recover_install_transaction(project.path()).unwrap();
        assert_eq!(fs::read(manifest).unwrap(), old_manifest);
        assert_eq!(fs::read(lock_path).unwrap(), old_lock);
        assert!(!vendor.exists());
    }
}
