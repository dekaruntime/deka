//! Validated build manifest (deka#719): the one host-owned record of what
//! Dsc compiled, which build-only values Deka materialized, which routes will
//! render, and which artifacts publish. The printed route table derives from
//! this artifact only — never from a second source scan.
//!
//! Lifecycle:
//! 1. [`BuildManifest::plan`] — validate the compiler plan and plan routes,
//!    BEFORE any build entry executes or any route renders.
//! 2. [`BuildManifest::expand_static_params`] — fill concrete `●` instances
//!    from the materialized values (post-execution, pre-render).
//! 3. render into a staging tree, then [`BuildManifest::record_artifacts`]
//!    hashes every published file.
//! 4. persist (`write`) and print (`render_route_table`) after the staged
//!    tree has atomically replaced `dist/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::manifest::{FrameworkEntry, FrameworkEntryKind, FrameworkManifest};

/// Manifest schema version.
pub const BUILD_MANIFEST_VERSION: u32 = 1;
/// Compiler plan versions this host accepts. v2 (dsc#54) adds the literal
/// `prerender` disposition; a v1 plan carries no route disposition.
pub const SUPPORTED_PLAN_VERSIONS: &[u32] = &[1, 2];

/// One source file's compiler plan, paired with the file it was produced for
/// (a plan's slots carry their file, but a file with no build slots still has
/// route disposition facts — `prerender` — so the pairing lives here).
#[derive(Debug, Clone)]
pub struct PlannedSource {
    pub file: String,
    pub plan: BuildPlan,
}

/// Versioned compiler-to-host contract (dsc#47, dsc#54). Owned here so the
/// plan shape has one spelling across the host (`cli` consumes this type).
#[derive(Debug, Clone, Deserialize)]
pub struct BuildPlan {
    pub version: u32,
    /// Literal `export const prerender = <bool>` disposition of the planned
    /// file. Absent on plan version 1.
    #[serde(default)]
    pub prerender: Option<bool>,
    pub slots: Vec<BuildPlanSlot>,
}

/// One materialization request in a [`BuildPlan`].
#[derive(Debug, Clone, Deserialize)]
pub struct BuildPlanSlot {
    pub id: String,
    pub binding: String,
    pub file: String,
    #[allow(dead_code)]
    pub span: serde_json::Value,
    /// Private compiler-owned descriptor. Deka validates materialized values
    /// against it but does not reimplement DekaScript type walking (rfd#48).
    pub descriptor: serde_json::Value,
    #[allow(dead_code)]
    pub entry: String,
}

/// Who produced the plan we validated against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerProvenance {
    pub plan_version: u32,
    /// `dsc --version` output when the binary reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsc: Option<String>,
}

/// One materialized build slot, recorded for cache validation and `deka dev`
/// invalidation (observations arrive with deka#725).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSlot {
    pub id: String,
    pub binding: String,
    /// Project-root-relative source file, forward-slash separated
    /// (`app/posts/page.dsx`); never absolute (deka#738 F2 — the manifest
    /// must be byte-identical across checkout roots).
    pub file: String,
    /// SHA-256 of the canonical descriptor JSON: identity for "the compiler
    /// approved this shape" without a second typechecker.
    pub descriptor_digest: String,
    /// Cache-only virtual module the value materialized into.
    pub value_module: String,
    /// Filesystem observations for targeted `deka dev` rebuilds (deka#725).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<FsObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsObservation {
    pub path: String,
    pub kind: FsObservationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsObservationKind {
    Read,
    Absent,
    DirectoryListing,
}

/// Route delivery mode; the glyphs are the route-table convention (deka#717).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// `○` fully static page.
    Static,
    /// `●` static page expanded from `staticParams`.
    StaticParams,
    /// `ƒ` request-time page (`prerender = false`).
    RequestTime,
    /// `λ` API handler.
    Api,
}

impl RouteMode {
    pub fn glyph(self) -> &'static str {
        match self {
            RouteMode::Static => "○",
            RouteMode::StaticParams => "●",
            RouteMode::RequestTime => "ƒ",
            RouteMode::Api => "λ",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            RouteMode::Static => "static",
            RouteMode::StaticParams => "static: staticParams",
            RouteMode::RequestTime => "prerender = false",
            RouteMode::Api => "api",
        }
    }
}

/// One row of the route plan, keyed by template: exactly one entry per
/// route, no matter how many `●` instances `staticParams` expands to
/// (deka#738 F6 — a two-param route must not record its template twice).
/// [`BuildManifest::render_route_table`] and the static render tasks expand
/// `instances` back into one row/task per concrete path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoute {
    /// Normalized route template, e.g. `/posts/[slug]`.
    pub template: String,
    /// Concrete paths for `●` rows, e.g. `["/posts/hello", "/posts/world"]`,
    /// sorted. Empty for every other mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<String>,
    pub mode: RouteMode,
    /// Project-root-relative source file, forward-slash separated
    /// (`app/page.dsx`); never absolute (deka#738 F2).
    pub source_file: String,
    /// `staticParams` provenance: the build slot id that supplied instances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// `server:defer` component names on this route's page (consumed by
    /// deka#718's `◐` classification).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred: Vec<String>,
}

/// One published file under `dist/`, hashed for the deterministic-artifact
/// contract (deka#720).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestArtifact {
    /// Forward-slash path relative to the `dist/` root, e.g.
    /// `client/posts/hello/index.html`.
    pub path: String,
    pub digest: String,
}

/// The validated build manifest. Field order is the canonical serialization
/// order; `routes`, `slots`, and `artifacts` are kept sorted so identical
/// inputs produce identical bytes (deka#720's determinism contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildManifest {
    pub version: u32,
    pub compiler: CompilerProvenance,
    pub slots: Vec<ManifestSlot>,
    pub routes: Vec<ManifestRoute>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ManifestArtifact>,
}

/// SHA-256 hex digest.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Reject unsupported, malformed, or internally inconsistent plans BEFORE
/// any build entry executes or any route renders. Slot ids must be unique
/// across the whole build, not just within one file's plan.
pub fn validate_plans(planned: &[PlannedSource]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for source in planned {
        let plan = &source.plan;
        if !SUPPORTED_PLAN_VERSIONS.contains(&plan.version) {
            return Err(format!(
                "unsupported dsc build plan version {}; deka supports versions {}",
                plan.version,
                SUPPORTED_PLAN_VERSIONS
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for slot in &plan.slots {
            if slot.id.is_empty() || slot.binding.is_empty() || slot.entry.is_empty() {
                return Err(
                    "dsc emitted a build plan slot with a missing id, binding, or entry"
                        .to_string(),
                );
            }
            if !ids.insert(&slot.id) {
                return Err(format!("dsc emitted duplicate build slot id `{}`", slot.id));
            }
        }
    }
    Ok(())
}

/// Normalize a path (absolute or project-relative) to a project-relative,
/// forward-slash form for comparison. Unknown/absurd inputs are returned
/// trimmed rather than rejected — this is a comparison key, not a validator.
fn project_relative(project_root: &Path, path: &str) -> String {
    let cleaned = path.replace('\\', "/");
    let as_path = Path::new(path);
    if as_path.is_absolute() {
        if let Ok(rel) = as_path.strip_prefix(project_root) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    cleaned.trim_start_matches("./").to_string()
}

/// Bracketed parameter names of a route template, in segment order. Mirrors
/// the v1 constraint enforced in `routes::assert_dynamic_route_supported`:
/// at most one `[param]`, as the last segment.
fn bracket_params(template: &str) -> Vec<String> {
    let mut params = Vec::new();
    for segment in template.trim_matches('/').split('/') {
        if let Some(name) = segment
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
        {
            params.push(name.to_string());
        }
    }
    params
}

/// Validate that a `staticParams` slot's descriptor can drive route
/// expansion: `Array<struct>` whose string fields exactly match the route's
/// bracket names. The descriptor is compiler-owned; we check shape, not
/// types in depth (values are validated against it at materialization).
fn validate_params_descriptor(
    template: &str,
    slot: &BuildPlanSlot,
    params: &[String],
) -> Result<(), String> {
    let descriptor = &slot.descriptor;
    if descriptor.get("node").and_then(|v| v.as_str()) != Some("array") {
        return Err(format!(
            "route {template}: staticParams (slot `{}`) must be an array of parameters",
            slot.id
        ));
    }
    let elem = descriptor
        .get("elem")
        .ok_or_else(|| format!("route {template}: staticParams descriptor has no elem"))?;
    if elem.get("node").and_then(|v| v.as_str()) != Some("struct") {
        return Err(format!(
            "route {template}: staticParams elements must be structs whose fields name the route parameters",
        ));
    }
    let fields = elem
        .get("fields")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("route {template}: staticParams descriptor has no fields"))?;
    let mut names = Vec::new();
    for field in fields {
        let name = field
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("route {template}: staticParams field is unnamed"))?;
        let kind = field
            .get("ty")
            .and_then(|ty| ty.get("node"))
            .and_then(|v| v.as_str());
        if kind != Some("leaf") || field.pointer("/ty/kind").and_then(|v| v.as_str()) != Some("string")
        {
            return Err(format!(
                "route {template}: staticParams field `{name}` must be a string (route parameters are path segments)",
            ));
        }
        names.push(name.to_string());
    }
    if names != params {
        return Err(format!(
            "route {template}: staticParams fields [{}] do not match the route parameters [{}]",
            names.join(", "),
            params.join(", ")
        ));
    }
    Ok(())
}

impl BuildManifest {
    /// Phase 1: validate the plans and plan every route, before execution.
    ///
    /// `app`/`api` are the filesystem scans; `planned` holds one compiler
    /// plan per source file (route disposition is per-file). `dsc_identity`
    /// is `dsc --version` output when available. `StaticParams` routes are
    /// recorded with no instances yet; call [`Self::expand_static_params`]
    /// after materialization.
    pub fn plan(
        project_root: &Path,
        planned: &[PlannedSource],
        dsc_identity: Option<String>,
        app: &FrameworkManifest,
        api: &[FrameworkEntry],
    ) -> Result<Self, String> {
        validate_plans(planned)?;
        let all_slots: Vec<&BuildPlanSlot> =
            planned.iter().flat_map(|source| source.plan.slots.iter()).collect();
        // Per-file route disposition: `prerender = false` as reported by
        // each file's plan (v2+). A v1 plan entry means "no disposition".
        let prerender_by_file: BTreeMap<String, Option<bool>> = planned
            .iter()
            .map(|source| {
                (
                    project_relative(project_root, &source.file),
                    source.plan.prerender.filter(|_| source.plan.version >= 2),
                )
            })
            .collect();
        let plan_version = planned
            .iter()
            .map(|source| source.plan.version)
            .max()
            .unwrap_or(1);

        let slot_file = |slot: &BuildPlanSlot| project_relative(project_root, &slot.file);

        let mut slots: Vec<ManifestSlot> = all_slots
            .iter()
            .map(|slot| ManifestSlot {
                id: slot.id.clone(),
                binding: slot.binding.clone(),
                file: project_relative(project_root, &slot.file),
                descriptor_digest: sha256_hex(slot.descriptor.to_string().as_bytes()),
                value_module: format!("deka:dev/{}", slot.id),
                observations: Vec::new(),
            })
            .collect();
        slots.sort_by(|a, b| a.id.cmp(&b.id));

        let mut routes: Vec<ManifestRoute> = Vec::new();

        // Pages, sorted by template for canonical output.
        let mut pages: Vec<&FrameworkEntry> = app
            .entries
            .iter()
            .filter(|entry| entry.kind == FrameworkEntryKind::Page)
            .collect();
        pages.sort_by(|a, b| a.route.cmp(&b.route).then(a.file.cmp(&b.file)));

        let mut seen_templates = BTreeSet::new();
        for page in &pages {
            let template = page.route.clone();
            if !seen_templates.insert(template.clone()) {
                return Err(format!(
                    "route collision: two app pages produce `{template}` ({})",
                    page.file
                ));
            }
            let file = project_relative(project_root, &page.file);
            let page_slots: Vec<&&BuildPlanSlot> = all_slots
                .iter()
                .filter(|slot| slot_file(slot) == file)
                .collect();
            let params_slot = page_slots
                .iter()
                .find(|slot| slot.binding == "staticParams");
            let params = bracket_params(&template);
            let prerender_false = prerender_by_file.get(&file) == Some(&Some(false));

            let route = match (params.is_empty(), params_slot, prerender_false) {
                (true, None, false) => ManifestRoute {
                    template,
                    instances: Vec::new(),
                    mode: RouteMode::Static,
                    source_file: file.clone(),
                    slot: None,
                    deferred: Vec::new(),
                },
                (true, None, true) => ManifestRoute {
                    template,
                    instances: Vec::new(),
                    mode: RouteMode::RequestTime,
                    source_file: file.clone(),
                    slot: None,
                    deferred: Vec::new(),
                },
                (true, Some(slot), _) => {
                    return Err(format!(
                        "route {template}: staticParams (slot `{}`) is declared but the route has no [parameter]",
                        slot.id
                    ));
                }
                (false, Some(slot), false) => {
                    validate_params_descriptor(&template, slot, &params)?;
                    ManifestRoute {
                        template,
                        instances: Vec::new(),
                        mode: RouteMode::StaticParams,
                        source_file: file.clone(),
                        slot: Some(slot.id.clone()),
                        deferred: Vec::new(),
                    }
                }
                (false, Some(slot), true) => {
                    return Err(format!(
                        "route {template}: declares both staticParams (slot `{}`) and prerender = false; pick one disposition",
                        slot.id
                    ));
                }
                (false, None, true) => ManifestRoute {
                    template,
                    instances: Vec::new(),
                    mode: RouteMode::RequestTime,
                    source_file: file.clone(),
                    slot: None,
                    deferred: Vec::new(),
                },
                (false, None, false) => {
                    // A v1 plan carries no disposition: the route may well
                    // declare one the compiler just cannot see yet.
                    let upgrade = if plan_version < 2 {
                        " (the installed dsc emits plan version 1, which reports no route disposition; upgrade dsc)"
                    } else {
                        ""
                    };
                    return Err(format!(
                        "route {template} is dynamic but declares neither staticParams nor prerender = false; a dynamic route must pick one{upgrade}"
                    ));
                }
            };
            routes.push(route);
        }

        // A staticParams slot in a file that is not an app page can only mean
        // confusion (wrong filename, wrong directory): fail rather than let
        // the value materialize into nothing.
        for slot in &all_slots {
            if slot.binding != "staticParams" {
                continue;
            }
            let file = slot_file(slot);
            let owns_page = pages
                .iter()
                .any(|page| project_relative(project_root, &page.file) == file);
            if !owns_page {
                return Err(format!(
                    "staticParams (slot `{}`) is declared in {}, which is not an app page; move it into the route's page module",
                    slot.id, slot.file
                ));
            }
        }

        // API routes are request-time handlers; they publish no static HTML
        // and live outside the output-collision map.
        let mut api_entries: Vec<&FrameworkEntry> = api
            .iter()
            .filter(|entry| entry.kind == FrameworkEntryKind::Api)
            .collect();
        api_entries.sort_by(|a, b| a.route.cmp(&b.route).then(a.file.cmp(&b.file)));
        for entry in api_entries {
            routes.push(ManifestRoute {
                template: entry.route.clone(),
                instances: Vec::new(),
                mode: RouteMode::Api,
                source_file: project_relative(project_root, &entry.file),
                slot: None,
                deferred: Vec::new(),
            });
        }

        // Canonical route order: templates are unique, so a plain template
        // sort fully determines the row order.
        routes.sort_by(|a, b| a.template.cmp(&b.template));

        Ok(BuildManifest {
            version: BUILD_MANIFEST_VERSION,
            compiler: CompilerProvenance {
                plan_version: plan_version,
                dsc: dsc_identity,
            },
            slots,
            routes,
            artifacts: Vec::new(),
        })
    }

    /// Phase 2: fill each `●` route's `instances` with the concrete paths
    /// from the materialized build values (`slot id -> value JSON`) — one
    /// manifest entry per template, however many instances it expands to
    /// (deka#738 F6). Must run after build entries execute (values are
    /// validated against the compiler descriptor there) and before
    /// rendering.
    pub fn expand_static_params(
        &mut self,
        values: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        let mut output_claims: BTreeMap<String, String> = BTreeMap::new();

        // Static and request-time routes claim their own template path.
        for route in &self.routes {
            if route.mode != RouteMode::StaticParams {
                if let Some(existing) = output_claims.insert(route.template.clone(), route.template.clone()) {
                    // Unreachable today (templates deduped in plan), kept as a
                    // guard for routes added by later phases.
                    return Err(format!(
                        "route collision: `{existing}` and `{}` publish the same path",
                        route.template
                    ));
                }
            }
        }

        for route in &mut self.routes {
            if route.mode != RouteMode::StaticParams {
                continue;
            }
            let slot_id = route
                .slot
                .as_ref()
                .expect("StaticParams route recorded without its slot id")
                .clone();
            let value = values.get(&slot_id).ok_or_else(|| {
                format!(
                    "route {}: staticParams value for slot `{}` was not materialized",
                    route.template, slot_id
                )
            })?;
            let elements = value
                .as_array()
                .ok_or_else(|| format!("route {}: materialized staticParams is not an array", route.template))?;
            let params = bracket_params(&route.template);
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut instances: Vec<String> = Vec::new();
            for element in elements {
                let mut instance = route.template.clone();
                for param in &params {
                    let raw = element
                        .get(param)
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            format!(
                                "route {}: materialized staticParams entry is missing string field `{param}`",
                                route.template
                            )
                        })?;
                    // Dot segments are rejected explicitly: the filesystem
                    // normalizes them, so "." and ".." would collide with
                    // sibling routes or write OUTSIDE dist/client (deka#719
                    // review, codex).
                    if raw.contains('/')
                        || raw.contains('\\')
                        || raw.is_empty()
                        || raw == "."
                        || raw == ".."
                    {
                        return Err(format!(
                            "route {}: parameter `{param}` value `{raw}` is not a single safe path segment",
                            route.template
                        ));
                    }
                    instance = instance.replace(&format!("[{param}]"), raw);
                }
                if !seen.insert(instance.clone()) {
                    return Err(format!(
                        "route collision: `{}` appears twice in staticParams for `{}`",
                        instance, route.template
                    ));
                }
                if let Some(existing) = output_claims.insert(instance.clone(), route.template.clone()) {
                    return Err(format!(
                        "route collision: `{instance}` from route `{}` collides with route `{existing}`",
                        route.template
                    ));
                }
                instances.push(instance);
            }
            if elements.is_empty() {
                return Err(format!(
                    "route {}: staticParams materialized to an empty array; the route would publish nothing",
                    route.template
                ));
            }
            // Concrete paths print and render in sorted order — this is the
            // canonical order the route table derives from.
            instances.sort();
            route.instances = instances;
        }

        self.routes.sort_by(|a, b| a.template.cmp(&b.template));
        Ok(())
    }

    /// Phase 3: hash every file under the staged dist tree. `staged_dist` is
    /// the directory that will become `dist/`; paths are recorded relative
    /// to it, forward-slash separated, sorted.
    pub fn record_artifacts(&mut self, staged_dist: &Path) -> Result<(), String> {
        let mut artifacts = Vec::new();
        if staged_dist.is_dir() {
            let mut stack = vec![staged_dist.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = std::fs::read_dir(&dir).map_err(|err| {
                    format!("failed to read {}: {err}", dir.display())
                })?;
                for entry in entries {
                    let entry = entry.map_err(|err| format!("read_dir entry error: {err}"))?;
                    let path = entry.path();
                    let file_type = entry
                        .file_type()
                        .map_err(|err| format!("file_type error for {}: {err}", path.display()))?;
                    if file_type.is_dir() {
                        stack.push(path);
                    } else if file_type.is_file() {
                        let rel = path
                            .strip_prefix(staged_dist)
                            .expect("walked path is under staged_dist")
                            .to_string_lossy()
                            .replace('\\', "/");
                        let bytes = std::fs::read(&path)
                            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
                        artifacts.push(ManifestArtifact {
                            path: rel,
                            digest: sha256_hex(&bytes),
                        });
                    }
                }
            }
        }
        artifacts.sort_by(|a, b| a.path.cmp(&b.path));
        self.artifacts = artifacts;
        Ok(())
    }

    /// Verify the published `dist/` tree against the recorded artifact
    /// digests (deka#738 F3): recompute sha256 for every recorded artifact
    /// against the on-disk bytes under `<project_root>/dist`. Returns one
    /// human-readable description per problem — a missing file or a digest
    /// mismatch, each naming the `dist/`-relative path — in recorded
    /// (path-sorted) order. An empty result means `dist/` matches the
    /// manifest byte for byte. This is the shared check behind `deka verify`;
    /// its semantics mirror the test harness's digest verification
    /// (deka#728).
    pub fn verify_artifacts(&self, project_root: &Path) -> Vec<String> {
        let dist = project_root.join("dist");
        let mut problems = Vec::new();
        for artifact in &self.artifacts {
            let on_disk = dist.join(&artifact.path);
            let bytes = match std::fs::read(&on_disk) {
                Ok(bytes) => bytes,
                Err(err) => {
                    problems.push(format!(
                        "artifact `dist/{}` cannot be read: {err}",
                        artifact.path
                    ));
                    continue;
                }
            };
            let actual = sha256_hex(&bytes);
            if actual != artifact.digest {
                problems.push(format!(
                    "artifact `dist/{}` digest mismatch: manifest declares {}, on-disk bytes hash to {}",
                    artifact.path, artifact.digest, actual
                ));
            }
        }
        problems
    }

    /// Deterministic serialization: identical manifests produce identical
    /// bytes (deka#720's repeated-build contract).
    pub fn canonical_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|err| format!("failed to serialize build manifest: {err}"))
    }

    /// Persist for diagnostics and cache validation. Internal artifact, not
    /// user configuration.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = format!("{}\n", self.canonical_json()?);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        std::fs::write(path, json.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    /// The final build route table. Printed only after the staged tree has
    /// replaced `dist/`; this is the only display source. `●` routes print
    /// one row per concrete instance (the manifest stores one entry per
    /// template — deka#738 F6 — so the instances expand here).
    pub fn render_route_table(&self) -> String {
        let mut rows: Vec<(&str, String, &str)> = Vec::new();
        for route in &self.routes {
            if route.instances.is_empty() {
                rows.push((
                    route.mode.glyph(),
                    route.template.clone(),
                    route.mode.detail(),
                ));
            } else {
                for instance in &route.instances {
                    rows.push((
                        route.mode.glyph(),
                        instance.clone(),
                        route.mode.detail(),
                    ));
                }
            }
        }
        let width = rows
            .iter()
            .map(|(_, path, _)| path.chars().count())
            .max()
            .unwrap_or(0);
        let mut out = String::new();
        for (glyph, path, detail) in rows {
            out.push_str(&format!("{glyph} {path:<width$}  {detail}\n"));
        }
        out
    }
}


#[cfg(test)]
#[path = "build_manifest_tests.rs"]
mod tests;
