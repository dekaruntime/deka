//! Built-artifact manifest v2 (deka#743 phase 1 spec:
//! `docs/architecture/built-artifact-manifest-v2.md`).
//!
//! The deployment descriptor `deka build` writes into `dist/` and `deka
//! serve` / `deka run` / the Worker adapter read. This is the versioned
//! contract at the build/serve boundary; the JSON Schema
//! (`crates/runtime_core/schemas/artifact-manifest-v2.schema.json`) is its
//! machine-readable normative form and is strict by design.
//!
//! Differences from the v1 [`super::BuildManifest`] (the internal planning
//! record, still written to the compiler cache for `deka dev`): v2 lives
//! inside `dist/`, describes the server entry table and the route→entry
//! mapping, spells digests as `sha256:<hex>`, carries payload byte counts and
//! roles, and is anchored by the `build-manifest.sha256` sidecar (§3.1: the
//! digest is over the manifest file bytes, never over a re-serialization).
//!
//! `source_file` / `slots[].file` are DIAGNOSTIC ONLY (§2.1): no consumer may
//! resolve, read, stat, or fall back to them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::build_manifest::{FsObservation, RouteMode, sha256_hex};
use super::manifest::{
    FrameworkEntry, FrameworkEntryKind, FrameworkManifest, exported_http_methods,
};

/// The `format` value every v2 manifest carries; unknown formats are
/// "incompatible" (§5.4), never parsed leniently.
pub const ARTIFACT_FORMAT: &str = "deka.artifact@2";
/// The host-module contract version this binary implements. Bumps only when
/// the host-module contract itself changes (§7.2), not per deka release.
pub const RUNTIME_ABI: u64 = 1;
/// The module language/version the artifact graph is emitted as (§4.1).
pub const MODULE_FORMAT: &str = "esm2022";

/// Files that constitute the descriptor and its anchor; never payloads (§2.4).
const DESCRIPTOR_FILES: [&str; 3] = [
    "build-manifest.json",
    "build-manifest.sha256",
    "build-manifest.sig",
];

/// Digest spelling for v2: `<alg>:<lowercase-hex>`, alg fixed at sha256 (§2.3).
pub fn artifact_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEntryKind {
    Page,
    Api,
    Defer,
    NotFound,
}

/// One executable server entry (§2 `server.entries[]`). `module` is a
/// `dist/`-relative path that must resolve inside `dist/server/` (the
/// resolution jail) and be listed in `payloads`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    pub kind: ServerEntryKind,
    pub module: String,
    /// The export the loader calls: `default` for page/defer/not_found; for
    /// api entries the first (sorted) declared method — the authoritative
    /// list is `methods`, this field only records the convention.
    pub export: String,
    /// api entries only: uppercase HTTP methods, sorted. Empty otherwise.
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadRole {
    Client,
    Server,
}

/// One published file under `dist/` (§2.4): every regular file except the
/// three descriptor files. Paths are forward-slash, `dist/`-relative, unique,
/// and sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPayload {
    pub path: String,
    pub role: PayloadRole,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRoute {
    pub template: String,
    pub mode: RouteMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<String>,
    /// Prerendered HTML payloads for static / static_params routes.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// `server.entries[].id`; required (non-null) for request_time and api
    /// routes, null otherwise.
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred: Vec<String>,
    /// DIAGNOSTIC ONLY (§2.1): never resolved, read, stat'ed, or fallen back
    /// to by any consumer.
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSlot {
    pub id: String,
    pub binding: String,
    /// Source path. DIAGNOSTIC ONLY (§2.1).
    pub file: String,
    pub descriptor_digest: String,
    /// Ordinary artifact module; replaces v1's `deka:dev/<id>` virtual import.
    pub module: String,
    /// Always present; `[]` in authored manifests (§2.2) so the descriptor's
    /// bytes never depend on dev-only filesystem observations.
    pub observations: Vec<FsObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactClient {
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    pub trailing_slash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactServer {
    pub root: String,
    pub entries: Vec<ServerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactWorker {
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCompat {
    pub runtime_abi: u64,
    pub module_format: String,
    pub targets: Vec<String>,
    /// The ONLY legal bare specifiers in server modules (§4.3). The phase-2
    /// artifact vendors its ui runtime as relative modules, so the emitted
    /// graph carries no bare specifiers at all.
    pub host_imports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactProducer {
    pub deka: String,
    pub dsc: String,
    pub plan_version: u32,
}

/// `dist/build-manifest.json` v2. Field order is the canonical serialization
/// order (§2); every array is pre-sorted so identical inputs produce
/// byte-identical manifests (deka#720's contract extends to this file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactManifestV2 {
    pub format: String,
    pub origin: String,
    pub producer: ArtifactProducer,
    pub compat: ArtifactCompat,
    pub client: ArtifactClient,
    pub server: ArtifactServer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<ArtifactWorker>,
    pub routes: Vec<ArtifactRoute>,
    pub slots: Vec<ArtifactSlot>,
    pub payloads: Vec<ArtifactPayload>,
    pub payload_root: String,
}

/// `payload_root` (§3.2): sha256 over, for each payload in path order, the
/// bytes `<path>\0<digest>\n` with the spelled (`sha256:…`) digest.
pub fn compute_payload_root(payloads: &[ArtifactPayload]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for payload in payloads {
        hasher.update(payload.path.as_bytes());
        hasher.update([0]);
        hasher.update(payload.digest.as_bytes());
        hasher.update([b'\n']);
    }
    artifact_digest(&hasher.finalize())
}

/// The prerendered-HTML payload a concrete static route publishes under
/// `client/` (the same mapping `runtime::prerender` writes with).
pub fn client_output_path(route: &str) -> String {
    if route == "/" {
        return "client/index.html".to_string();
    }
    let mut path = String::from("client");
    for segment in route.trim_start_matches('/').split('/') {
        path.push('/');
        path.push_str(segment);
    }
    path.push_str("/index.html");
    path
}

/// Resolve the authored artifact for a command subject (§5.1). `dist/` is a
/// commitment once it exists: its descriptor cannot be silently removed to
/// recover source compilation. The caller owns the user-facing remedy because
/// `deka serve`, `deka run`, and the Worker adapter phrase it differently.
pub fn resolve_authored_artifact_root(subject: &Path) -> Result<Option<PathBuf>, String> {
    let direct = subject.join("build-manifest.json");
    if direct.is_file() {
        return Ok(Some(subject.to_path_buf()));
    }
    let nested_root = subject.join("dist");
    if nested_root.join("build-manifest.json").is_file() {
        return Ok(Some(nested_root));
    }
    let direct_dist = subject.file_name().and_then(|name| name.to_str()) == Some("dist");
    if nested_root.is_dir() || direct_dist {
        return Err(format!(
            "incomplete artifact: no build-manifest.json found in {}",
            (if nested_root.is_dir() {
                nested_root
            } else {
                subject.to_path_buf()
            })
            .display()
        ));
    }
    Ok(None)
}

impl ArtifactManifestV2 {
    /// Hash every regular file under `staged_dist` (the tree that will become
    /// `dist/`) into `payloads`. The three descriptor files are excluded (§2.4);
    /// symlinks and non-regular files are a build error — an artifact must
    /// survive a tar/zip round trip and a Worker upload.
    pub fn record_payloads(&mut self, staged_dist: &Path) -> Result<(), String> {
        let mut payloads = Vec::new();
        if staged_dist.is_dir() {
            let mut stack = vec![staged_dist.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = std::fs::read_dir(&dir)
                    .map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
                for entry in entries {
                    let entry = entry.map_err(|err| format!("read_dir entry error: {err}"))?;
                    let path = entry.path();
                    let file_type = entry
                        .file_type()
                        .map_err(|err| format!("file_type error for {}: {err}", path.display()))?;
                    if file_type.is_dir() {
                        stack.push(path);
                    } else if file_type.is_symlink() {
                        return Err(format!(
                            "dist/ contains a symlink ({}); an artifact must survive a tar/zip round trip",
                            path.display()
                        ));
                    } else if file_type.is_file() {
                        let rel = path
                            .strip_prefix(staged_dist)
                            .expect("walked path is under staged_dist")
                            .to_string_lossy()
                            .replace('\\', "/");
                        if DESCRIPTOR_FILES.contains(&rel.as_str()) {
                            continue;
                        }
                        let bytes = std::fs::read(&path)
                            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
                        payloads.push(ArtifactPayload {
                            role: payload_role(&rel),
                            path: rel,
                            bytes: bytes.len() as u64,
                            digest: artifact_digest(&bytes),
                        });
                    } else {
                        return Err(format!(
                            "dist/ contains a non-regular file ({}); an artifact must survive a tar/zip round trip",
                            path.display()
                        ));
                    }
                }
            }
        }
        payloads.sort_by(|a, b| a.path.cmp(&b.path));
        self.payload_root = compute_payload_root(&payloads);
        self.payloads = payloads;
        Ok(())
    }

    /// Deterministic serialization: compact JSON, struct field order, exactly
    /// one trailing newline when written (§2).
    pub fn canonical_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|err| format!("failed to serialize artifact manifest: {err}"))
    }

    /// Write `build-manifest.json` and its `build-manifest.sha256` sidecar
    /// into `dist/`. The sidecar digests the manifest file bytes exactly as
    /// written on disk, trailing newline included (§3.1) — no canonicalization
    /// on the read side ever happens either.
    pub fn write_into(&self, dist_root: &Path) -> Result<(), String> {
        let manifest_path = dist_root.join("build-manifest.json");
        let json = format!("{}\n", self.canonical_json()?);
        std::fs::write(&manifest_path, json.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", manifest_path.display()))?;
        let sidecar = format!("{}  build-manifest.json\n", sha256_hex(json.as_bytes()));
        let sidecar_path = dist_root.join("build-manifest.sha256");
        std::fs::write(&sidecar_path, sidecar.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", sidecar_path.display()))?;
        Ok(())
    }

    /// Read `dist/build-manifest.json` and verify every boot-critical part of
    /// the artifact in §3.4 order. This is deliberately the only reader used
    /// while resolving an authored artifact: a caller must never inspect a
    /// source marker after this function reports an artifact fault.
    ///
    /// Client payloads other than the harness are verified by
    /// [`Self::read_client_payload`] on first use. This keeps a large client
    /// tree out of the startup critical path without ever serving unchecked
    /// bytes.
    pub fn load_verified(dist_root: &Path) -> Result<Self, String> {
        let manifest_path = dist_root.join("build-manifest.json");
        let bytes = std::fs::read(&manifest_path).map_err(|err| {
            format!(
                "no readable build manifest at {}: {err}; run `deka build` first",
                manifest_path.display()
            )
        })?;
        let sidecar_path = dist_root.join("build-manifest.sha256");
        let sidecar = std::fs::read_to_string(&sidecar_path).map_err(|err| {
            format!(
                "incomplete artifact: {} has no integrity anchor at {}: {err}",
                manifest_path.display(),
                sidecar_path.display()
            )
        })?;
        let expected = format!("{}  build-manifest.json\n", sha256_hex(&bytes));
        if sidecar != expected {
            return Err(format!(
                "tampered artifact: {} fails its integrity anchor check ({})",
                manifest_path.display(),
                sidecar_path.display()
            ));
        }
        let manifest: Self = serde_json::from_slice(&bytes)
            .map_err(|err| format!("invalid {}: {err}", manifest_path.display()))?;
        if manifest.format != ARTIFACT_FORMAT {
            return Err(format!(
                "incompatible artifact: format {:?} is not {ARTIFACT_FORMAT}",
                manifest.format
            ));
        }
        if manifest.compat.runtime_abi != RUNTIME_ABI {
            return Err(format!(
                "incompatible artifact: dist/ was built for runtime_abi {}; \
                 this deka implements {RUNTIME_ABI}. Rebuild with the matching toolchain: deka build",
                manifest.compat.runtime_abi
            ));
        }
        let recomputed = compute_payload_root(&manifest.payloads);
        if recomputed != manifest.payload_root {
            return Err(format!(
                "tampered artifact: payload_root mismatch (manifest declares {}, \
                 the payload table hashes to {})",
                manifest.payload_root, recomputed
            ));
        }
        for entry in &manifest.server.entries {
            if !entry.module.starts_with("server/") {
                return Err(format!(
                    "invalid artifact: server entry `{}` resolves outside server/ ({})",
                    entry.id, entry.module
                ));
            }
            manifest.require_declared_payload(dist_root, &entry.module, "server entry")?;
        }
        for slot in &manifest.slots {
            if !slot.module.starts_with("server/") {
                return Err(format!(
                    "invalid artifact: build value module `{}` resolves outside server/",
                    slot.module
                ));
            }
            manifest.require_declared_payload(dist_root, &slot.module, "build value module")?;
        }
        if let Some(worker) = &manifest.worker {
            manifest.require_declared_payload(dist_root, &worker.entry, "worker entry")?;
        }
        manifest.verify_startup_payloads(dist_root)?;
        manifest.verify_server_tree(dist_root)?;
        Ok(manifest)
    }

    /// Compatibility check for the native artifact loader. `producer` is
    /// intentionally informational; only the module ABI/format and declared
    /// target decide whether these bytes are executable here.
    pub fn ensure_native_compat(&self) -> Result<(), String> {
        if self.compat.module_format != MODULE_FORMAT {
            return Err(format!(
                "incompatible artifact: module_format {:?} is not {MODULE_FORMAT}",
                self.compat.module_format
            ));
        }
        if !self.compat.targets.iter().any(|target| target == "native") {
            return Err(
                "incompatible artifact: compat.targets does not include native".to_string(),
            );
        }
        Ok(())
    }

    /// Verify the payloads that can execute at boot: every server payload and
    /// the client harness. This is separate from [`Self::verify`], which is
    /// intentionally the eager `deka verify` whole-tree operation.
    fn verify_startup_payloads(&self, dist_root: &Path) -> Result<(), String> {
        for payload in &self.payloads {
            if payload.role == PayloadRole::Server
                || self.client.index.as_deref() == Some(payload.path.as_str())
            {
                self.verify_payload(dist_root, payload)?;
            }
        }
        if let Some(index) = &self.client.index {
            self.require_declared_payload(dist_root, index, "client index")?;
        }
        Ok(())
    }

    /// Read one client payload after checking that it is manifest-described
    /// and has the exact published bytes. `path` is always `dist/` relative.
    pub fn read_client_payload(&self, dist_root: &Path, path: &str) -> Result<Vec<u8>, String> {
        let payload = self
            .payloads
            .iter()
            .find(|payload| payload.path == path && payload.role == PayloadRole::Client)
            .ok_or_else(|| format!("artifact client payload `{path}` is not declared"))?;
        let bytes = std::fs::read(dist_root.join(path))
            .map_err(|err| format!("artifact client payload `{path}` cannot be read: {err}"))?;
        verify_payload_bytes(payload, &bytes)?;
        Ok(bytes)
    }

    fn verify_payload(&self, dist_root: &Path, payload: &ArtifactPayload) -> Result<(), String> {
        let bytes = std::fs::read(dist_root.join(&payload.path)).map_err(|err| {
            format!(
                "incomplete artifact: payload `{}` cannot be read: {err}",
                payload.path
            )
        })?;
        verify_payload_bytes(payload, &bytes)
    }

    /// The executable tree is a closed, manifest-described graph. A source
    /// file or an unlisted executable is not benign: either would let a
    /// source-style loader find bytes the manifest never authenticated.
    fn verify_server_tree(&self, dist_root: &Path) -> Result<(), String> {
        if self.server.root != "server" {
            return Err(format!(
                "invalid artifact: server.root must be `server`, got {:?}",
                self.server.root
            ));
        }
        let server_root = dist_root.join(&self.server.root);
        if !server_root.is_dir() {
            return Err(format!(
                "incomplete artifact: server root {} is missing; rebuild with `deka build`",
                server_root.display()
            ));
        }
        let declared: std::collections::BTreeSet<&str> = self
            .payloads
            .iter()
            .filter(|payload| payload.role == PayloadRole::Server)
            .map(|payload| payload.path.as_str())
            .collect();
        let mut stack = vec![server_root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).map_err(|err| {
                format!(
                    "failed to read artifact server tree {}: {err}",
                    dir.display()
                )
            })? {
                let entry = entry.map_err(|err| format!("artifact server tree entry: {err}"))?;
                let path = entry.path();
                let file_type = entry.file_type().map_err(|err| {
                    format!("artifact server file type {}: {err}", path.display())
                })?;
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    return Err(format!(
                        "invalid artifact: server tree contains a non-regular file {}",
                        path.display()
                    ));
                }
                let rel = path
                    .strip_prefix(dist_root)
                    .expect("server path is under dist")
                    .to_string_lossy()
                    .replace('\\', "/");
                if matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("ds" | "dsx")
                ) {
                    return Err(format!(
                        "invalid artifact: compiler input `{rel}` is present under dist/server; rebuild with `deka build`"
                    ));
                }
                if !declared.contains(rel.as_str()) {
                    return Err(format!(
                        "tampered artifact: server payload `{rel}` is not listed in build-manifest.json; rebuild with `deka build`"
                    ));
                }
            }
        }
        Ok(())
    }

    fn require_declared_payload(
        &self,
        dist_root: &Path,
        path: &str,
        what: &str,
    ) -> Result<(), String> {
        if !self.payloads.iter().any(|payload| payload.path == path) {
            return Err(format!(
                "invalid artifact: {what} `{path}` is not listed in payloads"
            ));
        }
        let on_disk = dist_root.join(path);
        if !on_disk.is_file() {
            return Err(format!(
                "incomplete artifact: {what} `{path}` is declared but missing on disk"
            ));
        }
        Ok(())
    }

    /// The eager full-tree check behind `deka verify`: hash every payload and
    /// compare against the recorded digest. Returns one human-readable
    /// description per problem, in path order.
    pub fn verify(&self, dist_root: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        for payload in &self.payloads {
            let on_disk = dist_root.join(&payload.path);
            let bytes = match std::fs::read(&on_disk) {
                Ok(bytes) => bytes,
                Err(err) => {
                    problems.push(format!(
                        "payload `dist/{}` cannot be read: {err}",
                        payload.path
                    ));
                    continue;
                }
            };
            if bytes.len() as u64 != payload.bytes {
                problems.push(format!(
                    "payload `dist/{}` size mismatch: manifest declares {} bytes, on-disk file has {}",
                    payload.path,
                    payload.bytes,
                    bytes.len()
                ));
            }
            let actual = artifact_digest(&bytes);
            if actual != payload.digest {
                problems.push(format!(
                    "payload `dist/{}` digest mismatch: manifest declares {}, on-disk bytes hash to {}",
                    payload.path, payload.digest, actual
                ));
            }
        }
        problems
    }
}

fn verify_payload_bytes(payload: &ArtifactPayload, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != payload.bytes {
        return Err(format!(
            "tampered artifact: payload `{}` size mismatch (manifest declares {} bytes, on-disk file has {})",
            payload.path,
            payload.bytes,
            bytes.len()
        ));
    }
    let actual = artifact_digest(bytes);
    if actual != payload.digest {
        return Err(format!(
            "tampered artifact: payload `{}` digest mismatch (manifest declares {}, on-disk bytes hash to {})",
            payload.path, payload.digest, actual
        ));
    }
    Ok(())
}

fn payload_role(rel: &str) -> PayloadRole {
    if rel.starts_with("server/") || rel == "_worker.js" {
        PayloadRole::Server
    } else {
        // client/** and root-level static deploy files (_redirects, …).
        PayloadRole::Client
    }
}

/// Build the `server.entries[]` table from the app/api scans. `file` fields
/// on the scans are absolute; they are relativized against `project_root` so
/// the manifest carries artifact paths (`server/app/page.js`), never host
/// paths (§2: the descriptor must be host-independent).
///
/// The native loader's compiled dispatch shims (`serve-entry.js`,
/// `api-entry.js`, `defer-entry.js`) are payload files like any other but are
/// deliberately NOT declared here: the schema's `kind` enum (page | api |
/// defer | not_found) and §4.7's entry contract (page/defer/not_found →
/// default export) cannot describe a router shim, and §4.7 makes a declared
/// export with no matching module export a verification failure. See the
/// deka#762 discussion on the spec gap.
pub fn server_entries(
    project_root: &Path,
    app: &FrameworkManifest,
    api: &[FrameworkEntry],
) -> Result<Vec<ServerEntry>, String> {
    let mut entries: Vec<ServerEntry> = Vec::new();

    let mut pages: Vec<&FrameworkEntry> = app
        .entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Page)
        .collect();
    pages.sort_by(|a, b| a.route.cmp(&b.route).then(a.file.cmp(&b.file)));
    for page in &pages {
        entries.push(ServerEntry {
            id: format!("page:{}", page.route),
            kind: ServerEntryKind::Page,
            module: source_to_server_module(&relativize(project_root, &page.file))?,
            export: "default".to_string(),
            methods: Vec::new(),
        });
    }
    if let Some(not_found) = &app.not_found {
        entries.push(ServerEntry {
            id: "not_found:/".to_string(),
            kind: ServerEntryKind::NotFound,
            module: source_to_server_module(&relativize(project_root, &not_found.file))?,
            export: "default".to_string(),
            methods: Vec::new(),
        });
    }

    let mut api_entries: Vec<&FrameworkEntry> = api
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Api)
        .collect();
    api_entries.sort_by(|a, b| a.route.cmp(&b.route).then(a.file.cmp(&b.file)));
    for api_entry in &api_entries {
        let methods = exported_http_methods(std::path::Path::new(&api_entry.file));
        if methods.is_empty() {
            continue;
        }
        let export = methods.first().cloned().expect("methods is non-empty");
        entries.push(ServerEntry {
            id: format!("api:{}", api_entry.route),
            kind: ServerEntryKind::Api,
            module: source_to_server_module(&relativize(project_root, &api_entry.file))?,
            export,
            methods,
        });
    }

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

/// `/abs/app/page.dsx` → `app/page.dsx` when under `project_root`; already
/// relative paths pass through.
fn relativize(project_root: &Path, file: &str) -> String {
    let path = Path::new(file);
    path.strip_prefix(project_root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.replace('\\', "/"))
}

/// Map a project source file (`app/posts/page.dsx`) to its compiled artifact
/// module path (`server/app/posts/page.js`).
pub fn source_to_server_module(file: &str) -> Result<String, String> {
    let rel = file.replace('\\', "/");
    let rel = rel.strip_prefix("./").unwrap_or(&rel).to_string();
    let stem = rel
        .strip_suffix(".dsx")
        .or_else(|| rel.strip_suffix(".ds"))
        .ok_or_else(|| format!("server entry source `{file}` is not a .ds/.dsx file"))?;
    Ok(format!("server/{stem}.js"))
}

#[cfg(test)]
#[path = "artifact_manifest_tests.rs"]
mod tests;
