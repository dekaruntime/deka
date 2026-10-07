//! Native server entry selection and content-addressed source compilation.
use crate::{Payload, Source, embedded, source};
use deka_vm::{HostOp, HostReply, HostValue, Hosts, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Default)]
pub(crate) struct Options {
    pub hostname: Option<String>,
    pub port: Option<u16>,
}
impl Options {
    pub fn supplied(&self) -> bool {
        self.hostname.is_some() || self.port.is_some()
    }
    pub fn arguments(argv: &[String]) -> Result<Self> {
        let mut options = Self::default();
        let mut args = argv.iter();
        while let Some(flag) = args.next() {
            let value = args.next().ok_or("expected server option value")?;
            match flag.as_str() {
                "--hostname" if options.hostname.is_none() => {
                    options.hostname = Some(value.clone())
                }
                "--port" if options.port.is_none() => {
                    options.port = Some(
                        value
                            .parse()
                            .map_err(|_| "port must be an integer from 0 to 65535")?,
                    )
                }
                _ => return Err("unexpected server arguments".into()),
            }
        }
        Ok(options)
    }
}
pub(crate) fn register(hosts: &mut Hosts, options: Options) -> Result<()> {
    hosts.register(HostOp::new(
        "__cli_http_options",
        vec![],
        deka_vm::http_server::options_type(),
        false,
        move |_| {
            let mut fields = BTreeMap::new();
            if let Some(host) = &options.hostname {
                fields.insert("hostname".into(), HostValue::String(host.clone()));
            }
            if let Some(port) = options.port {
                fields.insert("port".into(), HostValue::Number(port.into()));
            }
            HostReply::Ready(Ok(HostValue::Record(fields)))
        },
    ))?;
    hosts.register(HostOp::new(
        "__cli_http_listening",
        vec![deka_vm::tcp::address_type()],
        deka_vm::HostType::Unit,
        false,
        |args| {
            let HostValue::Record(fields) = &args[0] else {
                unreachable!("checked address")
            };
            let HostValue::String(host) = &fields["hostname"] else {
                unreachable!()
            };
            let HostValue::Number(port) = fields["port"] else {
                unreachable!()
            };
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.clone()
            };
            eprintln!("Listening on http://{host}:{port}/");
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
}
fn cache_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DEKA_CACHE_DIR") {
        return Ok(path.into());
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(path).join("deka"));
    }
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or("cannot locate the Deka cache: set DEKA_CACHE_DIR")?,
    )
    .join(".deka/cache"))
}
fn cached(source: &Source) -> Result<Payload> {
    // Include resolver inputs and this executable: two development builds may
    // share a version while exposing different bytecode or host contracts.
    let inputs = deka_vm::compiler::ScriptInputs::load(&source.path)?;
    let compile_snapshot = || -> Result<Payload> {
        let (program, server) = inputs.compile(&crate::hosts()?, source.entry.as_deref())?;
        Ok(Payload {
            version: 1,
            program,
            desktop: false,
            server,
        })
    };
    let mut hash = Sha256::new();
    hash.update(b"native-script-cache-v1");
    hash.update(b"sources");
    hash.update((inputs.sources().len() as u64).to_le_bytes());
    for (path, text) in inputs.sources() {
        for data in [path.as_os_str().as_encoded_bytes(), text.as_bytes()] {
            hash.update((data.len() as u64).to_le_bytes());
            hash.update(data);
        }
    }
    let resolutions = inputs.resolutions();
    hash.update(b"routes");
    hash.update((resolutions.len() as u64).to_le_bytes());
    for edge in resolutions {
        for data in [
            edge.importer.as_os_str().as_encoded_bytes(),
            edge.specifier.as_bytes(),
            edge.target.as_os_str().as_encoded_bytes(),
        ] {
            hash.update((data.len() as u64).to_le_bytes());
            hash.update(data);
        }
    }
    hash.update(serde_json::to_vec(&source.entry).map_err(|e| e.to_string())?);
    hash.update(
        fs::read(std::env::current_exe().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
    );
    let dir = cache_directory()?.join("native");
    let path = dir.join(format!("{:x}.json", hash.finalize()));
    match fs::read(&path) {
        Ok(bytes) => {
            if let Ok(payload) = decode_cache(&bytes) {
                return Ok(payload);
            }
            // Runtime-owned caches regenerate; authored applications never do.
            remove_entry(&path)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let payload = compile_snapshot()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.persist(&path).map_err(|e| e.to_string())?;
    evict(&dir, &path)?;
    decode_cache(&fs::read(&path).map_err(|e| e.to_string())?)
}
pub(crate) fn load(path: &Path, entry: Option<String>, serve: bool) -> Result<Payload> {
    // An explicitly authored executable wins over source and cache. Errors in
    // that artifact are reported before any listener is opened.
    let authored = if path.is_dir() {
        Some(path.join("dist/deka-app"))
    } else if path.file_name().is_some_and(|n| n == "deka.json") {
        Some(
            path.parent()
                .unwrap_or(Path::new("."))
                .join("dist/deka-app"),
        )
    } else if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("ds" | "dsx")
    ) {
        Some(path.to_owned())
    } else {
        None
    };
    let payload = if let Some(app) = authored.filter(|app| app.exists()) {
        if entry.is_some() {
            return Err("--entry cannot override a compiled application".into());
        }
        embedded(&app)
            .map_err(|e| format!("invalid authored application {}: {e}", app.display()))?
            .ok_or("authored application has no native Deka payload")?
    } else {
        let input = source(path, entry)?;
        if input.desktop {
            crate::compile(&input)?
        } else {
            cached(&input)?
        }
    };
    if serve && !payload.server {
        return Err("serve requires a default export with a fetch handler".into());
    }
    Ok(payload)
}

const CACHE_ENTRIES: usize = 64;
fn decode_cache(bytes: &[u8]) -> Result<Payload> {
    let payload: Payload = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    payload.program.validate()?;
    if payload.version != 1 || payload.desktop {
        return Err("invalid cached server application".into());
    }
    Ok(payload)
}
fn remove_entry(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
fn entries(dir: &Path) -> Result<Vec<(std::time::SystemTime, PathBuf)>> {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut entries = vec![];
    for entry in read {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(key) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
            continue;
        };
        if key.len() != 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        entries.push((
            metadata.modified().map_err(|e| e.to_string())?,
            entry.path(),
        ));
    }
    entries.sort();
    Ok(entries)
}
fn evict(dir: &Path, current: &Path) -> Result<()> {
    let files = entries(dir)?;
    let count = files.len().saturating_sub(CACHE_ENTRIES);
    for (_, path) in files
        .into_iter()
        .filter(|(_, path)| path != current)
        .take(count)
    {
        remove_entry(&path)?;
    }
    Ok(())
}
pub(crate) fn clear_cache() -> Result<usize> {
    let files = entries(&cache_directory()?.join("native"))?;
    let count = files.len();
    for (_, path) in files {
        remove_entry(&path)?;
    }
    Ok(count)
}
