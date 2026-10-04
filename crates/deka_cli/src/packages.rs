//! Native package delivery, ported from pm's deka.gg/R2 path (note 06).
use deka_vm::{Result, package::Lock};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path},
};

const INDEX: &str = "https://deka.gg";
const CDN: &str = "https://pub-6d81db17678348abba85f93fde4b4400.r2.dev";
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static TEST_ENDPOINT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[derive(Deserialize)]
struct IndexEntry {
    versions: Vec<String>,
}
struct Registry {
    index: String,
    cdn: String,
    client: reqwest::blocking::Client,
}
impl Registry {
    fn new(index: &str, cdn: &str) -> Result<Self> {
        Ok(Self {
            index: index.to_owned(),
            cdn: cdn.to_owned(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?,
        })
    }
    fn production() -> Result<Self> {
        #[cfg(test)]
        if let Some(url) = TEST_ENDPOINT.with(|endpoint| endpoint.borrow().clone()) {
            return Self::new(&url, &url);
        }
        Self::new(INDEX, CDN)
    }
    fn archive(&self, name: &str, version: &str) -> Result<(String, Vec<u8>)> {
        let package = name.strip_prefix("@deka/").ok_or_else(|| {
            format!("unknown package {name}: the deka.gg index provides @deka/* packages")
        })?;
        let response = self
            .client
            .get(format!("{}/api/registry/{package}.json", self.index))
            .send()
            .map_err(|e| format!("registry lookup failed for {name}: {e}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(format!("unknown package {name}"));
        }
        let metadata: IndexEntry = response
            .error_for_status()
            .map_err(|e| format!("registry lookup failed for {name}: {e}"))?
            .json()
            .map_err(|e| format!("invalid registry metadata for {name}: {e}"))?;
        if !metadata.versions.iter().any(|pin| pin == version) {
            return Err(format!("unknown version {version} for package {name}"));
        }
        let url = format!("{}/{package}/{version}/{package}-{version}.tgz", self.cdn);
        let bytes = self.download(&url, name, version)?;
        Ok((url, bytes))
    }
    fn download(&self, url: &str, name: &str, version: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| format!("download failed for {name}@{version}: {e}"))?;
        let mut bytes = Vec::new();
        response
            .take(MAX_ARCHIVE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("download failed for {name}@{version}: {e}"))?;
        if bytes.len() as u64 > MAX_ARCHIVE {
            return Err(format!(
                "download failed for {name}@{version}: archive exceeds 64 MiB"
            ));
        }
        Ok(bytes)
    }
}

pub(crate) fn parse_spec(spec: &str) -> Result<(&str, &str)> {
    let (name, version) = spec
        .rsplit_once('@')
        .ok_or("expected a package name with an exact version")?;
    let parts: Vec<_> = name.split('/').collect();
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    let valid_name = match parts.as_slice() {
        [name] => valid_part(name),
        [scope, name] => scope.strip_prefix('@').is_some_and(valid_part) && valid_part(name),
        _ => false,
    };
    if !valid_name {
        return Err(format!("invalid package name {name}"));
    }
    if semver::Version::parse(version).is_err() {
        return Err(format!(
            "expected an exact version for {name}, got {version:?}"
        ));
    }
    Ok((name, version))
}

fn validate_digest(name: &str, expected: &str) -> Result<()> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "missing or invalid tarball SHA-256 for package {name}"
        ));
    }
    Ok(())
}
fn verify_digest(name: &str, bytes: &[u8], expected: &str) -> Result<()> {
    validate_digest(name, expected)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "checksum mismatch for package {name}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn check_package_file(path: &Path, reader: &mut impl Read) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let executable_extension = matches!(
        extension.as_str(),
        "js" | "mjs"
            | "cjs"
            | "wasm"
            | "exe"
            | "dll"
            | "so"
            | "dylib"
            | "node"
            | "o"
            | "obj"
            | "a"
            | "lib"
            | "rs"
            | "rlib"
            | "rmeta"
    ) || filename.split_once(".so.").is_some_and(|(_, version)| {
        !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    });
    let mut prefix = Vec::new();
    reader
        .take(8)
        .read_to_end(&mut prefix)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let executable_format = prefix.starts_with(b"\0asm")
        || prefix.starts_with(b"\x7fELF")
        || prefix.starts_with(b"MZ")
        || prefix.starts_with(b"!<arch>\n")
        || prefix.starts_with(b"!<thin>\n")
        || [
            [0xfe, 0xed, 0xfa, 0xce],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
            [0xca, 0xfe, 0xba, 0xbf],
            [0xbf, 0xba, 0xfe, 0xca],
        ]
        .iter()
        .any(|magic| prefix.starts_with(magic));
    if executable_extension || executable_format {
        return Err(format!(
            "package is not DekaScript only: {}",
            path.display()
        ));
    }
    Ok(())
}

fn extract(bytes: &[u8], destination: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut seen = BTreeSet::new();
    let mut expanded = 0u64;
    for entry in archive
        .entries()
        .map_err(|e| format!("invalid package archive: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("invalid package archive: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!("invalid package archive path {}", path.display()));
        }
        if path.components().any(|c| matches!(c, Component::Normal(n) if n == ".git" || n.to_string_lossy().starts_with("._"))) {
            if entry.header().entry_type().is_file() {
                check_package_file(&path, &mut entry)?;
            }
            continue;
        }
        if path.components().any(|c| matches!(c, Component::Normal(n) if n == "ds_modules" || n == "php_modules" || n == "node_modules")) {
            return Err(format!("package archive contains vendored modules: {}", path.display()));
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(format!(
                "package archive contains a link or special file: {}",
                path.display()
            ));
        }
        if kind.is_file() && !seen.insert(path.clone()) {
            return Err(format!("duplicate package archive file {}", path.display()));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("package archive size overflow")?;
        if expanded > MAX_ARCHIVE {
            return Err("package archive expands beyond 64 MiB".into());
        }
        if !entry
            .unpack_in(destination)
            .map_err(|e| format!("{}: {e}", path.display()))?
        {
            return Err(format!(
                "package archive path escapes destination: {}",
                path.display()
            ));
        }
        if kind.is_file() {
            let mut file = fs::File::open(destination.join(&path))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            check_package_file(&path, &mut file)?;
        }
    }
    Ok(())
}

fn validate_package(
    bytes: &[u8],
    package: &Path,
    name: &str,
    version: &str,
) -> Result<BTreeMap<String, String>> {
    extract(bytes, package).map_err(|error| format!("package {name}@{version}: {error}"))?;
    let package_manifest = read_json(&package.join("deka.json"))?;
    if package_manifest.get("name").and_then(Value::as_str) != Some(name)
        || package_manifest.get("version").and_then(Value::as_str) != Some(version)
    {
        return Err(format!("package manifest does not match {name}@{version}"));
    }
    package_entry(package, &package_manifest)?;
    dependency_pins(&package_manifest, name)
}

pub(crate) fn package_entry(package: &Path, manifest: &Value) -> Result<std::path::PathBuf> {
    let entry = manifest
        .get("entry")
        .and_then(Value::as_str)
        .unwrap_or("index.ds");
    let entry_path = Path::new(entry);
    if entry_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || !matches!(
            entry_path.extension().and_then(|s| s.to_str()),
            Some("ds" | "dsx")
        )
        || !package.join(entry_path).is_file()
    {
        let name = manifest
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>");
        return Err(format!(
            "package {name} has no valid DekaScript entry: {entry}"
        ));
    }
    Ok(package.join(entry_path))
}

pub(crate) fn dependency_pins(manifest: &Value, owner: &str) -> Result<BTreeMap<String, String>> {
    let Some(value) = manifest.get("dependencies") else {
        return Ok(BTreeMap::new());
    };
    let values = value
        .as_object()
        .ok_or_else(|| format!("invalid dependencies for package {owner}"))?;
    values
        .iter()
        .map(|(name, pin)| {
            let pin = pin
                .as_str()
                .ok_or_else(|| format!("invalid version for dependency {name} of {owner}"))?;
            parse_spec(&format!("{name}@{pin}"))?;
            Ok((name.clone(), pin.to_owned()))
        })
        .collect()
}

fn lock_dependencies(name: &str, metadata: &Value) -> Result<BTreeMap<String, String>> {
    // Legacy dependency-free entries used an empty array.
    if metadata
        .get("dependencies")
        .is_some_and(|d| d.as_array().is_some_and(Vec::is_empty))
    {
        return Ok(BTreeMap::new());
    }
    dependency_pins(metadata, name)
}

fn check_dependency_edges(
    name: &str,
    dependencies: &BTreeMap<String, String>,
    lock: &Lock,
) -> Result<()> {
    for (dependency, version) in dependencies {
        let entry = lock.packages.get(dependency).ok_or_else(|| {
            format!("package {name} dependency {dependency}@{version} is missing from deka.lock")
        })?;
        let pin = deka_vm::package::version_pin(dependency, &entry.0);
        if pin != version {
            return Err(format!(
                "package {name} dependency {dependency} version conflict: expects {version}, deka.lock pins {pin}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn add(directory: &Path, spec: &str) -> Result<()> {
    add_from(directory, spec, &Registry::production()?)
}
fn add_from(directory: &Path, spec: &str, registry: &Registry) -> Result<()> {
    let (name, version) = parse_spec(spec)?;
    let manifest_path = directory.join("deka.json");
    let lock_path = directory.join("deka.lock");
    let original_manifest =
        fs::read(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let mut manifest: Value = serde_json::from_slice(&original_manifest)
        .map_err(|e| format!("invalid deka.json: {e}"))?;
    if !manifest.is_object() {
        return Err("deka.json must be an object".into());
    }
    let original_lock = match fs::read(&lock_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", lock_path.display())),
    };
    let old_lock: Lock = match &original_lock {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|e| format!("invalid deka.lock: {e}"))?
        }
        None => Lock {
            version: 1,
            ..Lock::default()
        },
    };
    if old_lock.version != 1 {
        return Err(format!(
            "unsupported deka.lock version {}",
            old_lock.version
        ));
    }
    let dependencies = manifest
        .as_object_mut()
        .unwrap()
        .entry("dependencies")
        .or_insert_with(|| json!({}));
    let dependencies = dependencies
        .as_object_mut()
        .ok_or("deka.json dependencies must be an object")?;
    // Validate configuration before network access or filesystem mutation.
    for (dependency, pin) in dependencies.iter() {
        let pin = pin
            .as_str()
            .ok_or_else(|| format!("invalid version for {dependency}"))?;
        parse_spec(&format!("{dependency}@{pin}"))?;
    }
    dependencies.insert(name.into(), Value::String(version.into()));
    let mut pending: Vec<(String, String)> = dependencies
        .iter()
        .map(|(name, version)| (name.clone(), version.as_str().unwrap().to_owned()))
        .collect();
    // Refuse malformed hashes for known direct pins before any request.
    for (name, version) in &pending {
        if let Some((pin, _, _, hash)) = old_lock.packages.get(name)
            && deka_vm::package::version_pin(name, pin) == version
        {
            validate_digest(name, hash)?;
        }
    }
    let stage = tempfile::Builder::new()
        .prefix(".deka-add-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    let modules = stage.path().join("modules");
    fs::create_dir(&modules).map_err(|e| e.to_string())?;
    let mut lock = Lock::default();
    while let Some((name, version)) = pending.pop() {
        if let Some((chosen, _, _, _)) = lock.packages.get(&name) {
            if chosen != &version {
                return Err(format!(
                    "package {name} version conflict: exact pins {chosen} and {version}"
                ));
            }
            continue;
        }
        let expected = old_lock
            .packages
            .get(&name)
            .filter(|(pin, _, _, _)| deka_vm::package::version_pin(&name, pin) == version)
            .map(|(_, _, _, hash)| hash.as_str());
        if let Some(hash) = expected {
            validate_digest(&name, hash)?;
        }
        let (url, bytes) = registry.archive(&name, &version)?;
        if let Some(hash) = expected {
            verify_digest(&name, &bytes, hash)?;
        }
        let package = modules.join(&name);
        fs::create_dir_all(&package).map_err(|e| e.to_string())?;
        let children = validate_package(&bytes, &package, &name, &version)?;
        pending.extend(
            children
                .iter()
                .map(|(name, version)| (name.clone(), version.clone())),
        );
        lock.packages.insert(
            name,
            (
                version,
                url,
                json!({"dependencies":children}),
                format!("{:x}", Sha256::digest(&bytes)),
            ),
        );
    }
    let manifest_bytes = encode(&manifest)?;
    let lock_bytes = encode(&lock)?;
    let destination = directory.join("ds_modules");
    if destination.is_symlink() || (destination.exists() && !destination.is_dir()) {
        return Err(format!(
            "package parent is not a directory: {}",
            destination.display()
        ));
    }
    for name in lock.packages.keys() {
        let mut path = destination.clone();
        for component in Path::new(name).components() {
            path.push(component);
            if path.is_symlink() || (path.exists() && !path.is_dir()) {
                return Err(format!(
                    "package parent is not a directory: {}",
                    path.display()
                ));
            }
        }
    }
    let backup = stage.path().join("previous-package");
    let had_package = destination.exists();
    if had_package {
        fs::rename(&destination, &backup).map_err(|e| e.to_string())?;
    }
    let result = (|| -> std::io::Result<()> {
        fs::rename(&modules, &destination)?;
        fs::write(&manifest_path, manifest_bytes)?;
        fs::write(&lock_path, lock_bytes)?;
        Ok(())
    })();
    if let Err(error) = result {
        let rollback = (|| -> std::io::Result<()> {
            if destination.exists() {
                fs::remove_dir_all(&destination)?;
            }
            if had_package {
                fs::rename(&backup, &destination)?;
            }
            fs::write(&manifest_path, original_manifest)?;
            match original_lock {
                Some(bytes) => fs::write(&lock_path, bytes)?,
                None if lock_path.exists() => fs::remove_file(&lock_path)?,
                None => {}
            }
            Ok(())
        })();
        if let Err(rollback) = rollback {
            let retained = stage.keep();
            return Err(format!(
                "package add failed: {error}; rollback failed: {rollback}; recovery files at {}",
                retained.display()
            ));
        }
        return Err(format!("package add failed: {error}"));
    }
    Ok(())
}

pub(crate) fn install(directory: &Path) -> Result<usize> {
    install_from(directory, &Registry::production()?)
}

fn install_from(directory: &Path, registry: &Registry) -> Result<usize> {
    read_json(&directory.join("deka.json"))?;
    let lock: Lock = serde_json::from_slice(
        &fs::read(directory.join("deka.lock")).map_err(|e| format!("deka.lock: {e}"))?,
    )
    .map_err(|e| format!("invalid deka.lock: {e}"))?;
    if lock.version != 1 {
        return Err(format!("unsupported deka.lock version {}", lock.version));
    }
    for (name, (pin, url, _, expected)) in &lock.packages {
        let version = deka_vm::package::version_pin(name, pin);
        parse_spec(&format!("{name}@{version}"))?;
        validate_digest(name, expected)?;
        check_dependency_edges(
            name,
            &lock_dependencies(name, &lock.packages[name].2)?,
            &lock,
        )?;
        let url =
            reqwest::Url::parse(url).map_err(|e| format!("invalid tarball URL for {name}: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("invalid tarball URL for {name}: expected HTTP(S)"));
        }
    }
    let destination = directory.join("ds_modules");
    if destination.is_symlink() || (destination.exists() && !destination.is_dir()) {
        return Err("ds_modules must be a directory, not a file or symlink".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".deka-install-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    let modules = stage.path().join("modules");
    fs::create_dir(&modules).map_err(|e| e.to_string())?;
    for (name, (pin, url, _, expected)) in &lock.packages {
        let version = deka_vm::package::version_pin(name, pin);
        let bytes = registry.download(url, name, version)?;
        verify_digest(name, &bytes, expected)?;
        let package = modules.join(name);
        fs::create_dir_all(&package).map_err(|e| e.to_string())?;
        let dependencies = validate_package(&bytes, &package, name, version)?;
        if dependencies != lock_dependencies(name, &lock.packages[name].2)? {
            return Err(format!(
                "package {name} dependencies do not match deka.lock"
            ));
        }
        check_dependency_edges(name, &dependencies, &lock)?;
    }
    let backup = stage.path().join("previous-modules");
    let had_modules = destination.exists();
    if had_modules {
        fs::rename(&destination, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(&modules, &destination) {
        if had_modules && let Err(rollback) = fs::rename(&backup, &destination) {
            let retained = stage.keep();
            return Err(format!(
                "install failed: {error}; rollback failed: {rollback}; recovery files at {}",
                retained.display()
            ));
        }
        return Err(format!("install failed: {error}"));
    }
    Ok(lock.packages.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::{
        io::Write,
        net::{SocketAddr, TcpListener, TcpStream},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    struct Fixture {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        responses: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        stop: Arc<AtomicBool>,
        address: SocketAddr,
        thread: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    }
    impl Fixture {
        fn new(responses: BTreeMap<String, Vec<u8>>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let url = format!("http://{address}");
            listener.set_nonblocking(false).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let responses = Arc::new(Mutex::new(responses));
            let replies = responses.clone();
            let thread = std::thread::spawn(move || -> std::io::Result<()> {
                loop {
                    let (mut stream, _) = listener.accept()?;
                    if stopping.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    stream.set_nonblocking(false)?;
                    let timeout = Some(std::time::Duration::from_secs(5));
                    stream.set_read_timeout(timeout)?;
                    stream.set_write_timeout(timeout)?;
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte)?;
                        request.push(byte[0]);
                    }
                    let request = String::from_utf8(request)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    let path = request.split_whitespace().nth(1).ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request path")
                    })?;
                    seen.lock()
                        .map_err(|e| std::io::Error::other(e.to_string()))?
                        .push(path.into());
                    let body = replies
                        .lock()
                        .map_err(|e| std::io::Error::other(e.to_string()))?
                        .get(path)
                        .cloned();
                    let status = if body.is_some() {
                        "200 OK"
                    } else {
                        "404 Not Found"
                    };
                    let bytes = body.as_deref().unwrap_or(b"missing");
                    write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    )?;
                    stream.write_all(bytes)?;
                }
            });
            Self {
                url,
                requests,
                responses,
                stop,
                address,
                thread: Some(thread),
            }
        }
        fn registry(&self) -> Registry {
            Registry::new(&self.url, &self.url).unwrap()
        }
        fn command(
            &self,
            directory: &Path,
            args: &[&str],
        ) -> (std::process::ExitCode, String, String) {
            struct Restore(Option<String>);
            impl Drop for Restore {
                fn drop(&mut self) {
                    TEST_ENDPOINT.with(|endpoint| *endpoint.borrow_mut() = self.0.take());
                }
            }
            let _restore =
                Restore(TEST_ENDPOINT.with(|endpoint| endpoint.replace(Some(self.url.clone()))));
            let mut args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            args.extend(["--directory".into(), directory.display().to_string()]);
            let (code, out, err) = crate::cli::registry().run_captured(&args);
            (code, out.string().to_owned(), err.string().to_owned())
        }
    }
    impl Fixture {
        fn shutdown(&mut self) -> Result<()> {
            self.stop.store(true, Ordering::Release);
            let Some(thread) = self.thread.take() else {
                return Ok(());
            };
            if !thread.is_finished() {
                // Wake blocking accept, even when no HTTP request was made.
                if let Err(error) =
                    TcpStream::connect_timeout(&self.address, std::time::Duration::from_secs(5))
                    && !thread.is_finished()
                {
                    // Do not hang cleanup if the wake connection itself failed.
                    return Err(format!("fixture shutdown connection: {error}"));
                }
            }
            match thread.join() {
                Ok(result) => result.map_err(|error| format!("fixture server I/O: {error}")),
                Err(_) => Err("fixture server thread panicked".into()),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(error) = self.shutdown() {
                if std::thread::panicking() {
                    eprintln!("{error}");
                } else {
                    panic!("{error}");
                }
            }
        }
    }

    #[test]
    fn fixture_reports_incomplete_request_as_io_error() {
        let mut fixture = Fixture::new(BTreeMap::new());
        let mut stream = TcpStream::connect(fixture.address).unwrap();
        stream.write_all(b"GET /unfinished").unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        // Wait for the server's error before shutdown can cancel this request.
        for _ in 0..500 {
            if fixture.thread.as_ref().unwrap().is_finished() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let error = fixture.shutdown().unwrap_err();
        assert!(error.starts_with("fixture server I/O:"), "{error}");
    }
    fn tarball(name: &str, version: &str, entry: &str) -> Vec<u8> {
        archive_files([
            (
                "deka.json",
                encode(&json!({"name": name, "version": version, "entry": "index.ds"})).unwrap(),
            ),
            ("index.ds", entry.as_bytes().to_vec()),
        ])
    }
    fn archive_files(files: impl IntoIterator<Item = (&'static str, Vec<u8>)>) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        for (path, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes.as_slice())
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn real_commands_refuse_executable_files_and_preserve_live_packages() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        assert_eq!(
            fixture
                .command(project.path(), &["add", "@deka/demo@1.2.3"])
                .0,
            std::process::ExitCode::SUCCESS
        );
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let entry = project.path().join("ds_modules/@deka/demo/index.ds");
        let source = fs::read(&entry).unwrap();
        for (path, bytes) in [
            ("nested/legacy.js", b"export const answer = 0;".as_slice()),
            ("nested/legacy.MJS", b"export const answer = 0;".as_slice()),
            ("legacy.cjs", b"module.exports = {};".as_slice()),
            ("src/legacy.rs", b"fn main() {}".as_slice()),
            ("engine.wasm", b"\0asm\x01\0\0\0".as_slice()),
            ("native/plugin.so.1", b"native".as_slice()),
            ("native/plugin.dll", b"native".as_slice()),
            ("native/plugin.dylib", b"native".as_slice()),
            ("assets/renamed.dat", b"\x7fELFbinary".as_slice()),
            ("assets/macos.dat", b"\xcf\xfa\xed\xfebinary".as_slice()),
            ("assets/windows.dat", b"MZbinary".as_slice()),
            ("assets/wasm.dat", b"\0asm\x01\0\0\0".as_slice()),
            ("assets/archive.dat", b"!<arch>\nbinary".as_slice()),
            (".git/hidden.js", b"bad".as_slice()),
        ] {
            let archive = archive_files([
                (
                    "deka.json",
                    encode(&json!({"name":"@deka/demo","version":"2.0.0","entry":"index.ds"}))
                        .unwrap(),
                ),
                ("index.ds", b"export const answer = 99;".to_vec()),
                (path, bytes.to_vec()),
            ]);
            fixture.responses.lock().unwrap().extend([
                (
                    "/api/registry/demo.json".into(),
                    br#"{"versions":["1.2.3","2.0.0"]}"#.to_vec(),
                ),
                ("/demo/2.0.0/demo-2.0.0.tgz".into(), archive.clone()),
            ]);
            let (code, _, error) = fixture.command(project.path(), &["add", "@deka/demo@2.0.0"]);
            assert_eq!(code, std::process::ExitCode::from(1), "{path}: {error}");
            assert!(
                error.contains(&format!("not DekaScript only: {path}")),
                "{error}"
            );
            assert_eq!(fs::read(&entry).unwrap(), source);
            assert_eq!(
                fs::read(project.path().join("deka.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
            let mut replay: Lock = serde_json::from_slice(&lock).unwrap();
            let pin = replay.packages.get_mut("@deka/demo").unwrap();
            pin.0 = "2.0.0".into();
            pin.1 = format!("{}/demo/2.0.0/demo-2.0.0.tgz", fixture.url);
            pin.3 = format!("{:x}", Sha256::digest(&archive));
            let replay = encode(&replay).unwrap();
            fs::write(project.path().join("deka.lock"), &replay).unwrap();
            let (code, _, error) = fixture.command(project.path(), &["install"]);
            assert_eq!(code, std::process::ExitCode::from(1), "{path}: {error}");
            assert!(
                error.contains(&format!("not DekaScript only: {path}")),
                "{error}"
            );
            assert_eq!(fs::read(&entry).unwrap(), source);
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), replay);
            fs::write(project.path().join("deka.lock"), &lock).unwrap();
        }
    }

    #[test]
    fn source_packages_keep_non_executable_assets_on_add_and_install() {
        let archive = archive_files([
            (
                "deka.json",
                encode(&json!({"name":"@deka/demo","version":"1.2.3","entry":"index.ds"})).unwrap(),
            ),
            ("index.ds", b"export const answer = 42;".to_vec()),
            ("assets/data.json", br#"{"hello":"world"}"#.to_vec()),
            ("assets/icon.png", b"\x89PNG\r\n\x1a\n".to_vec()),
        ]);
        let fixture = Fixture::new(responses("demo", archive));
        let project = project();
        assert_eq!(
            fixture
                .command(project.path(), &["add", "@deka/demo@1.2.3"])
                .0,
            std::process::ExitCode::SUCCESS
        );
        fs::remove_dir_all(project.path().join("ds_modules")).unwrap();
        assert_eq!(
            fixture.command(project.path(), &["install"]).0,
            std::process::ExitCode::SUCCESS
        );
        assert_eq!(
            fs::read(
                project
                    .path()
                    .join("ds_modules/@deka/demo/assets/data.json")
            )
            .unwrap(),
            br#"{"hello":"world"}"#
        );
        assert_eq!(
            fs::read(project.path().join("ds_modules/@deka/demo/assets/icon.png")).unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
    }

    fn package_archive(name: &str, version: &str, entry: &str, dependencies: Value) -> Vec<u8> {
        archive_files([
            ("deka.json", encode(&json!({"name":name,"version":version,"entry":"index.ds","dependencies":dependencies})).unwrap()),
            ("index.ds", entry.as_bytes().to_vec()),
        ])
    }
    fn module_files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).unwrap().to_owned(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }
    fn run_consumer(source: &Path) -> deka_vm::HostValue {
        let hosts = deka_vm::Hosts::default();
        let program = deka_vm::compiler::compile_file(source, &hosts, Some("main")).unwrap();
        let mut vm = deka_vm::Vm::new(program, hosts).unwrap();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(vm.run())
            .unwrap()
    }

    #[test]
    fn public_package_errors_name_the_failure_and_preserve_live_files() {
        let original = tarball("@deka/demo", "1.2.3", "export const answer = 42;");
        let fixture = Fixture::new(responses("demo", original));
        let project = project();
        assert_eq!(
            fixture
                .command(project.path(), &["add", "@deka/demo@1.2.3"])
                .0,
            std::process::ExitCode::SUCCESS
        );
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let files = module_files(&project.path().join("ds_modules"));
        let script = archive_files([
            (
                "deka.json",
                encode(&json!({"name":"@deka/demo","version":"2.0.0","entry":"index.ds"})).unwrap(),
            ),
            ("index.ds", b"export const answer = 99;".to_vec()),
            ("legacy.js", b"export const answer = 0;".to_vec()),
        ]);
        for (spec, versions, archive, needle) in [
            (
                "@deka/missing@1.2.3",
                br#"{"versions":["1.2.3"]}"#.as_slice(),
                None,
                "unknown package @deka/missing",
            ),
            (
                "@deka/demo@2.0.0",
                br#"{"versions":["1.2.3"]}"#.as_slice(),
                None,
                "unknown version 2.0.0 for package @deka/demo",
            ),
            (
                "@deka/demo@2.0.0",
                br#"{"versions":["1.2.3","2.0.0"]}"#.as_slice(),
                None,
                "download failed for @deka/demo@2.0.0",
            ),
            (
                "@deka/demo@1.2.3",
                br#"{"versions":["1.2.3"]}"#.as_slice(),
                Some(tarball("@deka/demo", "1.2.3", "export const answer = 666;")),
                "checksum mismatch for package @deka/demo",
            ),
            (
                "@deka/demo@2.0.0",
                br#"{"versions":["1.2.3","2.0.0"]}"#.as_slice(),
                Some(script),
                "package @deka/demo@2.0.0: package is not DekaScript only: legacy.js",
            ),
            (
                "@deka/demo@2.0.0",
                br#"{"versions":["1.2.3","2.0.0"]}"#.as_slice(),
                Some(b"not an archive".to_vec()),
                "package @deka/demo@2.0.0: invalid package archive",
            ),
        ] {
            let (_, version) = parse_spec(spec).unwrap();
            let path = format!("/demo/{version}/demo-{version}.tgz");
            let mut responses = fixture.responses.lock().unwrap();
            responses.insert("/api/registry/demo.json".into(), versions.to_vec());
            responses.remove(&path);
            if let Some(bytes) = &archive {
                responses.insert(path.clone(), bytes.clone());
            }
            drop(responses);
            let (code, _, error) = fixture.command(project.path(), &["add", spec]);
            assert_eq!(code, std::process::ExitCode::from(1), "{spec}: {error}");
            assert!(error.contains(needle), "{spec}: {error}");
            assert_eq!(module_files(&project.path().join("ds_modules")), files);
            assert_eq!(
                fs::read(project.path().join("deka.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
            // Replay failures use recorded URLs, so lookup/version errors are add-only.
            if needle.starts_with("download failed") || archive.is_some() {
                let mut replay: Lock = serde_json::from_slice(&lock).unwrap();
                let pin = replay.packages.get_mut("@deka/demo").unwrap();
                pin.0 = version.into();
                pin.1 = format!("{}{path}", fixture.url);
                if !needle.starts_with("checksum mismatch") {
                    pin.3 = archive
                        .as_ref()
                        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                        .unwrap_or_else(|| "0".repeat(64));
                }
                let replay = encode(&replay).unwrap();
                fs::write(project.path().join("deka.lock"), &replay).unwrap();
                let (code, _, error) = fixture.command(project.path(), &["install"]);
                assert_eq!(code, std::process::ExitCode::from(1));
                assert!(error.contains(needle), "{error}");
                assert_eq!(module_files(&project.path().join("ds_modules")), files);
                assert_eq!(
                    fs::read(project.path().join("deka.json")).unwrap(),
                    manifest
                );
                assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), replay);
                fs::write(project.path().join("deka.lock"), &lock).unwrap();
            }
        }
    }

    #[test]
    fn transitive_diamond_executes_and_replays_without_exposing_root_imports() {
        let mut replies = BTreeMap::new();
        for (name, source, dependencies) in [
            (
                "top",
                "import { left } from \"@deka/left\"; import { right } from \"@deka/right\"; export fn answer() number { return left() + right(); }",
                json!({"@deka/left":"1.2.3","@deka/right":"1.2.3"}),
            ),
            (
                "left",
                "import { value } from \"@deka/leaf/index.ds\"; export fn left() number { return value(); }",
                json!({"@deka/leaf":"1.2.3"}),
            ),
            (
                "right",
                "import { value } from \"@deka/leaf/index.ds\"; export fn right() number { return value(); }",
                json!({"@deka/leaf":"1.2.3"}),
            ),
            ("leaf", "export fn value() number { return 21; }", json!({})),
        ] {
            replies.extend(responses(
                name,
                package_archive(&format!("@deka/{name}"), "1.2.3", source, dependencies),
            ));
        }
        let fixture = Fixture::new(replies);
        let project = project();
        let (code, _, error) = fixture.command(project.path(), &["add", "@deka/top@1.2.3"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        let source = project.path().join("main.ds");
        fs::write(
            &source,
            "import { answer } from \"@deka/top\"; fn main() number { return answer(); }",
        )
        .unwrap();
        assert_eq!(run_consumer(&source), deka_vm::HostValue::Number(42.));
        let (code, _, error) =
            crate::cli::registry().run_captured(&["check".into(), source.display().to_string()]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{}", error.string());
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let parsed: Lock = serde_json::from_slice(&lock).unwrap();
        assert_eq!(parsed.packages.len(), 4);
        assert_eq!(
            parsed.packages["@deka/top"].2["dependencies"]["@deka/left"],
            "1.2.3"
        );
        assert_eq!(
            parsed.packages["@deka/left"].2["dependencies"]["@deka/leaf"],
            "1.2.3"
        );
        assert_eq!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.ends_with("leaf-1.2.3.tgz"))
                .count(),
            1
        );
        fs::write(
            &source,
            "import { value } from \"@deka/leaf\"; fn main() number { return value(); }",
        )
        .unwrap();
        let error =
            deka_vm::compiler::compile_file(&source, &deka_vm::Hosts::default(), Some("main"))
                .err()
                .unwrap();
        assert!(error.contains("@deka/leaf is not declared"), "{error}");
        fs::write(
            &source,
            "import { answer } from \"@deka/top\"; fn main() number { return answer(); }",
        )
        .unwrap();
        let before = module_files(&project.path().join("ds_modules"));
        fs::remove_dir_all(project.path().join("ds_modules")).unwrap();
        fixture.requests.lock().unwrap().clear();
        let (code, _, error) = fixture.command(project.path(), &["install"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        assert_eq!(module_files(&project.path().join("ds_modules")), before);
        assert_eq!(
            fs::read(project.path().join("deka.json")).unwrap(),
            manifest
        );
        assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
        assert_eq!(fixture.requests.lock().unwrap().len(), 4);
        assert!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|path| !path.starts_with("/api/"))
        );
        assert_eq!(run_consumer(&source), deka_vm::HostValue::Number(42.));
        // A subpath import must check the same exact pin as a package-root import.
        let leaf_manifest = project.path().join("ds_modules/@deka/leaf/deka.json");
        let mut leaf = read_json(&leaf_manifest).unwrap();
        leaf["version"] = json!("2.0.0");
        fs::write(leaf_manifest, encode(&leaf).unwrap()).unwrap();
        let error =
            deka_vm::compiler::compile_file(&source, &deka_vm::Hosts::default(), Some("main"))
                .err()
                .unwrap();
        assert!(error.contains("version mismatch"), "{error}");
    }

    #[test]
    fn dependency_cycles_resolve_each_archive_once() {
        let mut replies = responses(
            "a",
            package_archive(
                "@deka/a",
                "1.2.3",
                "export const a = 7;",
                json!({"@deka/b":"1.2.3"}),
            ),
        );
        replies.extend(responses(
            "b",
            package_archive(
                "@deka/b",
                "1.2.3",
                "export const b = 9;",
                json!({"@deka/a":"1.2.3"}),
            ),
        ));
        let fixture = Fixture::new(replies);
        let project = project();
        let (code, _, error) = fixture.command(project.path(), &["add", "@deka/a@1.2.3"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        let lock: Lock =
            serde_json::from_slice(&fs::read(project.path().join("deka.lock")).unwrap()).unwrap();
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(fixture.requests.lock().unwrap().len(), 4);
        assert_eq!(
            lock.packages["@deka/b"].2["dependencies"]["@deka/a"],
            "1.2.3"
        );
        assert_eq!(
            fixture.command(project.path(), &["install"]).0,
            std::process::ExitCode::SUCCESS
        );
    }

    #[test]
    fn transitive_failure_conflicts_and_invalid_locks_preserve_the_live_graph() {
        let mut replies = responses(
            "top",
            package_archive(
                "@deka/top",
                "1.2.3",
                "export const top = 7;",
                json!({"@deka/leaf":"1.2.3"}),
            ),
        );
        replies.extend(responses(
            "leaf",
            tarball("@deka/leaf", "1.2.3", "export const leaf = 9;"),
        ));
        let fixture = Fixture::new(replies);
        let project = project();
        assert_eq!(
            fixture
                .command(project.path(), &["add", "@deka/top@1.2.3"])
                .0,
            std::process::ExitCode::SUCCESS
        );
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let modules = module_files(&project.path().join("ds_modules"));
        fixture.responses.lock().unwrap().extend([
            (
                "/api/registry/leaf.json".into(),
                br#"{"versions":["1.2.3","2.0.0"]}"#.to_vec(),
            ),
            (
                "/leaf/2.0.0/leaf-2.0.0.tgz".into(),
                tarball("@deka/leaf", "2.0.0", "export const leaf = 99;"),
            ),
        ]);
        for (dependencies, needle) in [
            (
                json!({"@deka/missing":"1.2.3"}),
                "unknown package @deka/missing",
            ),
            (json!({"@deka/leaf":"2.0.0"}), "@deka/leaf version conflict"),
            (json!({"@deka/leaf":"^1.2.3"}), "expected an exact version"),
        ] {
            fixture.responses.lock().unwrap().extend(responses(
                "bad",
                package_archive("@deka/bad", "1.2.3", "export const bad = 0;", dependencies),
            ));
            let (code, _, error) = fixture.command(project.path(), &["add", "@deka/bad@1.2.3"]);
            assert_eq!(code, std::process::ExitCode::from(1));
            assert!(error.contains(needle), "{error}");
            assert_eq!(module_files(&project.path().join("ds_modules")), modules);
            assert_eq!(
                fs::read(project.path().join("deka.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
        }
        for (mut broken, needle) in [
            (
                serde_json::from_slice::<Lock>(&lock).unwrap(),
                "missing from deka.lock",
            ),
            (
                serde_json::from_slice::<Lock>(&lock).unwrap(),
                "version conflict",
            ),
            (
                serde_json::from_slice::<Lock>(&lock).unwrap(),
                "dependencies do not match deka.lock",
            ),
        ] {
            if needle == "missing from deka.lock" {
                broken.packages.remove("@deka/leaf");
            } else if needle == "version conflict" {
                broken.packages.get_mut("@deka/leaf").unwrap().0 = "2.0.0".into();
            } else {
                broken.packages.get_mut("@deka/top").unwrap().2 = json!({"dependencies":{}});
            }
            let broken = encode(&broken).unwrap();
            fs::write(project.path().join("deka.lock"), &broken).unwrap();
            let (code, _, error) = fixture.command(project.path(), &["install"]);
            assert_eq!(code, std::process::ExitCode::from(1));
            assert!(error.contains(needle), "{error}");
            assert_eq!(module_files(&project.path().join("ds_modules")), modules);
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), broken);
        }
        fs::write(project.path().join("deka.lock"), &lock).unwrap();
        fixture.responses.lock().unwrap().extend([
            (
                "/api/registry/top.json".into(),
                br#"{"versions":["1.2.3","2.0.0"]}"#.to_vec(),
            ),
            (
                "/top/2.0.0/top-2.0.0.tgz".into(),
                tarball("@deka/top", "2.0.0", "export const top = 99;"),
            ),
        ]);
        let (code, _, error) = fixture.command(project.path(), &["add", "@deka/top@2.0.0"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        assert!(!project.path().join("ds_modules/@deka/leaf").exists());
        assert_eq!(
            read_json(&project.path().join("deka.lock")).unwrap()["packages"]
                .as_object()
                .unwrap()
                .len(),
            1
        );
    }

    fn responses(name: &str, archive: Vec<u8>) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (
                format!("/api/registry/{name}.json"),
                br#"{"versions":["1.2.3"]}"#.to_vec(),
            ),
            (format!("/{name}/1.2.3/{name}-1.2.3.tgz"), archive),
        ])
    }
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("deka.json"),
            br#"{"name":"consumer","entry":"main.ds","desktop":{"custom":7}}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn exact_add_downloads_old_registry_shape_and_runs_the_native_consumer() {
        let bytes = tarball(
            "@deka/demo",
            "1.2.3",
            "export fn answer() number { return 42; }",
        );
        let fixture = Fixture::new(responses("demo", bytes.clone()));
        let project = project();
        add_from(project.path(), "@deka/demo@1.2.3", &fixture.registry()).unwrap();
        let manifest = read_json(&project.path().join("deka.json")).unwrap();
        assert_eq!(manifest["desktop"]["custom"], 7);
        assert_eq!(manifest["dependencies"]["@deka/demo"], "1.2.3");
        let lock = read_json(&project.path().join("deka.lock")).unwrap();
        assert_eq!(lock["packages"]["@deka/demo"][0], "1.2.3");
        assert_eq!(
            lock["packages"]["@deka/demo"][3],
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert_eq!(
            *fixture.requests.lock().unwrap(),
            ["/api/registry/demo.json", "/demo/1.2.3/demo-1.2.3.tgz"]
        );
        let source = project.path().join("main.ds");
        fs::write(
            &source,
            "import { answer } from \"@deka/demo\"; fn main() number { return answer(); }",
        )
        .unwrap();
        let hosts = deka_vm::Hosts::default();
        let program = deka_vm::compiler::compile_file(&source, &hosts, Some("main")).unwrap();
        let mut vm = deka_vm::Vm::new(program, hosts).unwrap();
        let output = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(vm.run())
            .unwrap();
        assert_eq!(output, deka_vm::HostValue::Number(42.));
    }

    #[test]
    fn adding_a_second_package_retains_existing_pins_and_release_bytes() {
        let mut reply = responses(
            "first",
            tarball("@deka/first", "1.2.3", "export const first = 7;"),
        );
        reply.extend(responses(
            "second",
            tarball("@deka/second", "1.2.3", "export const second = 9;"),
        ));
        let fixture = Fixture::new(reply);
        let project = project();
        add_from(project.path(), "@deka/first@1.2.3", &fixture.registry()).unwrap();
        let first_lock =
            read_json(&project.path().join("deka.lock")).unwrap()["packages"]["@deka/first"]
                .clone();
        let first_bytes = fs::read(project.path().join("ds_modules/@deka/first/index.ds")).unwrap();
        add_from(project.path(), "@deka/second@1.2.3", &fixture.registry()).unwrap();
        let manifest = read_json(&project.path().join("deka.json")).unwrap();
        assert_eq!(manifest["dependencies"].as_object().unwrap().len(), 2);
        assert_eq!(
            read_json(&project.path().join("deka.lock")).unwrap()["packages"]["@deka/first"],
            first_lock
        );
        assert_eq!(
            fs::read(project.path().join("ds_modules/@deka/first/index.ds")).unwrap(),
            first_bytes
        );
    }

    #[test]
    fn rejected_downloads_and_manifests_preserve_an_existing_install() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        add_from(project.path(), "@deka/demo@1.2.3", &fixture.registry()).unwrap();
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let file = fs::read(project.path().join("ds_modules/@deka/demo/index.ds")).unwrap();
        let wrong = Fixture::new(responses(
            "demo",
            tarball("@deka/wrong", "1.2.3", "export const answer = 0;"),
        ));
        for (spec, registry, needle) in [
            ("@deka/demo@1.2.3", wrong.registry(), "checksum mismatch"),
            ("@deka/demo@2.0.0", fixture.registry(), "unknown version"),
            ("@deka/missing@1.2.3", fixture.registry(), "unknown package"),
        ] {
            assert!(
                add_from(project.path(), spec, &registry)
                    .unwrap_err()
                    .contains(needle)
            );
            assert_eq!(
                fs::read(project.path().join("deka.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
            assert_eq!(
                fs::read(project.path().join("ds_modules/@deka/demo/index.ds")).unwrap(),
                file
            );
        }
    }

    #[test]
    fn invalid_specs_fail_before_requesting_the_registry() {
        let fixture = Fixture::new(BTreeMap::new());
        let project = project();
        for spec in [
            "@deka/demo",
            "@deka/demo@^1.2.3",
            "@deka/demo@latest",
            "../../escape@1.2.3",
            "@deka/demo/path@1.2.3",
        ] {
            assert!(add_from(project.path(), spec, &fixture.registry()).is_err());
        }
        assert!(fixture.requests.lock().unwrap().is_empty());
        assert!(!project.path().join("deka.lock").exists());
    }

    #[test]
    fn real_commands_reject_tampered_tarballs_without_changing_live_files() {
        let original = tarball("@deka/demo", "1.2.3", "export const answer = 42;");
        let fixture = Fixture::new(responses("demo", original.clone()));
        let project = project();
        let (code, _, err) = fixture.command(project.path(), &["add", "@deka/demo@1.2.3"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{err}");
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        let entry = project.path().join("ds_modules/@deka/demo/index.ds");
        let source = fs::read(&entry).unwrap();
        let url = "/demo/1.2.3/demo-1.2.3.tgz";
        fixture.responses.lock().unwrap().insert(
            url.into(),
            tarball("@deka/demo", "1.2.3", "export const answer = 666;"),
        );
        for args in [&["add", "@deka/demo@1.2.3"][..], &["install"][..]] {
            let (code, _, error) = fixture.command(project.path(), args);
            assert_eq!(code, std::process::ExitCode::from(1));
            assert!(
                error.contains("checksum mismatch for package @deka/demo"),
                "{error}"
            );
            assert_eq!(fs::read(&entry).unwrap(), source);
            assert_eq!(
                fs::read(project.path().join("deka.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
        }
        fixture
            .responses
            .lock()
            .unwrap()
            .insert(url.into(), original);
        fs::remove_dir_all(project.path().join("ds_modules")).unwrap();
        let (code, _, error) = fixture.command(project.path(), &["install"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        assert_eq!(fs::read(&entry).unwrap(), source);
    }

    #[test]
    fn real_commands_refuse_missing_hashes_before_requests_and_allow_explicit_new_versions() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        assert_eq!(
            fixture
                .command(project.path(), &["add", "@deka/demo@1.2.3"])
                .0,
            std::process::ExitCode::SUCCESS
        );
        let saved = fs::read(project.path().join("deka.lock")).unwrap();
        let before = fixture.requests.lock().unwrap().len();
        for invalid in ["", "xyz", &"g".repeat(64)] {
            let mut lock: Lock = serde_json::from_slice(&saved).unwrap();
            lock.packages.get_mut("@deka/demo").unwrap().3 = invalid.into();
            fs::write(project.path().join("deka.lock"), encode(&lock).unwrap()).unwrap();
            for args in [&["add", "@deka/demo@1.2.3"][..], &["install"][..]] {
                let (code, _, error) = fixture.command(project.path(), args);
                assert_eq!(code, std::process::ExitCode::from(1));
                assert!(
                    error.contains("missing or invalid tarball SHA-256 for package @deka/demo"),
                    "{error}"
                );
            }
        }
        assert_eq!(fixture.requests.lock().unwrap().len(), before);
        fs::write(project.path().join("deka.lock"), saved).unwrap();
        let next = tarball("@deka/demo", "2.0.0", "export const answer = 99;");
        fixture.responses.lock().unwrap().extend([
            (
                "/api/registry/demo.json".into(),
                br#"{"versions":["1.2.3","2.0.0"]}"#.to_vec(),
            ),
            ("/demo/2.0.0/demo-2.0.0.tgz".into(), next.clone()),
        ]);
        let (code, _, error) = fixture.command(project.path(), &["add", "@deka/demo@2.0.0"]);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "{error}");
        let lock = read_json(&project.path().join("deka.lock")).unwrap();
        assert_eq!(lock["packages"]["@deka/demo"][0], "2.0.0");
        assert_eq!(
            lock["packages"]["@deka/demo"][3],
            format!("{:x}", Sha256::digest(&next))
        );
    }

    #[test]
    fn an_unlocked_first_download_still_checks_the_package_manifest() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/wrong", "1.2.3", "export const answer = 0;"),
        ));
        let project = project();
        let (code, _, error) = fixture.command(project.path(), &["add", "@deka/demo@1.2.3"]);
        assert_eq!(code, std::process::ExitCode::from(1));
        assert!(
            error.contains("package manifest does not match @deka/demo@1.2.3"),
            "{error}"
        );
        assert!(!project.path().join("deka.lock").exists());
        assert!(!project.path().join("ds_modules").exists());
    }

    #[test]
    fn replay_restores_identical_bytes_prunes_stale_files_and_never_resolves_versions() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        add_from(project.path(), "@deka/demo@1.2.3", &fixture.registry()).unwrap();
        let modules = project.path().join("ds_modules");
        let original = ["@deka/demo/deka.json", "@deka/demo/index.ds"]
            .map(|path| (path, fs::read(modules.join(path)).unwrap()));
        let manifest = fs::read(project.path().join("deka.json")).unwrap();
        let lock = fs::read(project.path().join("deka.lock")).unwrap();
        fs::remove_dir_all(&modules).unwrap();
        let before = fixture.requests.lock().unwrap().len();
        assert_eq!(
            install_from(project.path(), &fixture.registry()).unwrap(),
            1
        );
        for (path, bytes) in &original {
            assert_eq!(fs::read(modules.join(path)).unwrap(), *bytes);
        }
        fs::create_dir(modules.join("stale-package")).unwrap();
        fs::write(modules.join("stale-package/index.ds"), "wrong").unwrap();
        fs::write(modules.join("@deka/demo/untracked.txt"), "wrong").unwrap();
        install_from(project.path(), &fixture.registry()).unwrap();
        assert!(!modules.join("stale-package").exists());
        assert!(!modules.join("@deka/demo/untracked.txt").exists());
        for (path, bytes) in &original {
            assert_eq!(fs::read(modules.join(path)).unwrap(), *bytes);
        }
        assert_eq!(
            fs::read(project.path().join("deka.json")).unwrap(),
            manifest
        );
        assert_eq!(fs::read(project.path().join("deka.lock")).unwrap(), lock);
        assert_eq!(
            fixture.requests.lock().unwrap()[before..],
            ["/demo/1.2.3/demo-1.2.3.tgz", "/demo/1.2.3/demo-1.2.3.tgz"]
        );
    }

    #[test]
    fn a_later_failed_download_preserves_the_entire_previous_module_tree() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        add_from(project.path(), "@deka/demo@1.2.3", &fixture.registry()).unwrap();
        let modules = project.path().join("ds_modules");
        fs::write(modules.join("keep.txt"), "keep").unwrap();
        let source = fs::read(modules.join("@deka/demo/index.ds")).unwrap();
        let mut lock: Lock =
            serde_json::from_slice(&fs::read(project.path().join("deka.lock")).unwrap()).unwrap();
        lock.packages.insert(
            "@deka/zzz".into(),
            (
                "1.2.3".into(),
                format!("{}/missing.tgz", fixture.url),
                json!({}),
                "0".repeat(64),
            ),
        );
        fs::write(project.path().join("deka.lock"), encode(&lock).unwrap()).unwrap();
        let error = install_from(project.path(), &fixture.registry()).unwrap_err();
        assert!(
            error.contains("download failed for @deka/zzz@1.2.3"),
            "{error}"
        );
        assert_eq!(fs::read(modules.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(modules.join("@deka/demo/index.ds")).unwrap(),
            source
        );
        assert!(!modules.join("@deka/zzz").exists());
    }

    #[test]
    fn invalid_lock_paths_fail_before_network_access_or_module_replacement() {
        let fixture = Fixture::new(BTreeMap::new());
        let project = project();
        fs::create_dir(project.path().join("ds_modules")).unwrap();
        fs::write(project.path().join("ds_modules/keep.txt"), "keep").unwrap();
        for (name, version, url, diagnostic) in [
            (
                "../escape",
                "1.2.3",
                "https://example.invalid/a.tgz",
                "invalid package name",
            ),
            (
                "@deka/demo",
                "latest",
                "https://example.invalid/a.tgz",
                "expected an exact version",
            ),
            (
                "@deka/demo",
                "1.2.3",
                "file:///etc/passwd",
                "expected HTTP(S)",
            ),
        ] {
            let lock = json!({"lockfileVersion": 1, "packages": {name: [version, url, {}, "0".repeat(64)]}});
            fs::write(project.path().join("deka.lock"), encode(&lock).unwrap()).unwrap();
            let error = install_from(project.path(), &fixture.registry()).unwrap_err();
            assert!(error.contains(diagnostic), "{error}");
            assert_eq!(
                fs::read(project.path().join("ds_modules/keep.txt")).unwrap(),
                b"keep"
            );
        }
        assert!(fixture.requests.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_package_parent_cannot_redirect_an_install() {
        let fixture = Fixture::new(responses(
            "demo",
            tarball("@deka/demo", "1.2.3", "export const answer = 42;"),
        ));
        let project = project();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("ds_modules")).unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("ds_modules/@deka"))
            .unwrap();
        let before = fs::read(project.path().join("deka.json")).unwrap();
        let error = add_from(project.path(), "@deka/demo@1.2.3", &fixture.registry()).unwrap_err();
        assert!(
            error.contains("package parent is not a directory"),
            "{error}"
        );
        assert!(!outside.path().join("demo").exists());
        assert!(!project.path().join("deka.lock").exists());
        assert_eq!(fs::read(project.path().join("deka.json")).unwrap(), before);
    }
}
