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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// One row of the route plan. `instance` is set for `●` rows (one row per
/// expanded concrete path); every other mode has exactly one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoute {
    /// Normalized route template, e.g. `/posts/[slug]`.
    pub template: String,
    /// Concrete path for `●` rows, e.g. `/posts/hello`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub mode: RouteMode,
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
                file: slot.file.clone(),
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
                    instance: None,
                    mode: RouteMode::Static,
                    source_file: page.file.clone(),
                    slot: None,
                    deferred: Vec::new(),
                },
                (true, None, true) => ManifestRoute {
                    template,
                    instance: None,
                    mode: RouteMode::RequestTime,
                    source_file: page.file.clone(),
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
                        instance: None,
                        mode: RouteMode::StaticParams,
                        source_file: page.file.clone(),
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
                    instance: None,
                    mode: RouteMode::RequestTime,
                    source_file: page.file.clone(),
                    slot: None,
                    deferred: Vec::new(),
                },
                (false, None, false) => {
                    return Err(format!(
                        "route {template} is dynamic but declares neither staticParams nor prerender = false; a dynamic route must pick one"
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
                instance: None,
                mode: RouteMode::Api,
                source_file: entry.file.clone(),
                slot: None,
                deferred: Vec::new(),
            });
        }

        // Canonical route order: by concrete path when present (instance
        // rows), else template; then template as tiebreak so ● rows of one
        // route stay together.
        routes.sort_by(|a, b| {
            a.instance
                .as_deref()
                .unwrap_or(&a.template)
                .cmp(b.instance.as_deref().unwrap_or(&b.template))
                .then(a.template.cmp(&b.template))
        });

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

    /// Phase 2: expand `●` routes into concrete instances from the
    /// materialized build values (`slot id -> value JSON`). Must run after
    /// build entries execute (values are validated against the compiler
    /// descriptor there) and before rendering.
    pub fn expand_static_params(
        &mut self,
        values: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        let mut expanded: Vec<ManifestRoute> = Vec::new();
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

        for route in &self.routes {
            if route.mode != RouteMode::StaticParams {
                expanded.push(route.clone());
                continue;
            }
            let slot_id = route
                .slot
                .as_ref()
                .expect("StaticParams route recorded without its slot id");
            let value = values.get(slot_id).ok_or_else(|| {
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
                    if raw.contains('/') || raw.contains('\\') || raw.is_empty() {
                        return Err(format!(
                            "route {}: parameter `{param}` value `{raw}` is not a single path segment",
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
                expanded.push(ManifestRoute {
                    template: route.template.clone(),
                    instance: Some(instance),
                    mode: RouteMode::StaticParams,
                    source_file: route.source_file.clone(),
                    slot: route.slot.clone(),
                    deferred: route.deferred.clone(),
                });
            }
            if elements.is_empty() {
                return Err(format!(
                    "route {}: staticParams materialized to an empty array; the route would publish nothing",
                    route.template
                ));
            }
        }

        expanded.sort_by(|a, b| {
            a.instance
                .as_deref()
                .unwrap_or(&a.template)
                .cmp(b.instance.as_deref().unwrap_or(&b.template))
                .then(a.template.cmp(&b.template))
        });
        self.routes = expanded;
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
    /// replaced `dist/`; this is the only display source.
    pub fn render_route_table(&self) -> String {
        let rows: Vec<(&str, String, &str)> = self
            .routes
            .iter()
            .map(|route| {
                (
                    route.mode.glyph(),
                    route
                        .instance
                        .clone()
                        .unwrap_or_else(|| route.template.clone()),
                    route.mode.detail(),
                )
            })
            .collect();
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
mod tests {
    use super::*;
    use crate::framework::manifest::{FrameworkEntry, FrameworkManifest};

    fn page(template: &str, file: &str) -> FrameworkEntry {
        FrameworkEntry {
            kind: FrameworkEntryKind::Page,
            route: template.to_string(),
            file: file.to_string(),
        }
    }

    fn api(route: &str, file: &str) -> FrameworkEntry {
        FrameworkEntry {
            kind: FrameworkEntryKind::Api,
            route: route.to_string(),
            file: file.to_string(),
        }
    }

    fn params_descriptor(fields: &[(&str, &str)]) -> serde_json::Value {
        serde_json::json!({
            "node": "array",
            "elem": {
                "node": "struct",
                "fields": fields.iter().map(|(name, kind)| serde_json::json!({
                    "name": name,
                    "optional": false,
                    "ty": { "node": "leaf", "kind": kind, "name": kind }
                })).collect::<Vec<_>>()
            }
        })
    }

    fn slot(id: &str, binding: &str, file: &str, descriptor: serde_json::Value) -> BuildPlanSlot {
        BuildPlanSlot {
            id: id.to_string(),
            binding: binding.to_string(),
            file: file.to_string(),
            span: serde_json::json!({}),
            descriptor,
            entry: "export default async function __deka_dev_x() {}".to_string(),
        }
    }

    fn plan(version: u32, slots: Vec<BuildPlanSlot>) -> BuildPlan {
        BuildPlan {
            version,
            prerender: None,
            slots,
        }
    }

    fn planned(file: &str, plan: BuildPlan) -> PlannedSource {
        PlannedSource {
            file: file.to_string(),
            plan,
        }
    }

    fn app(pages: &[FrameworkEntry]) -> FrameworkManifest {
        FrameworkManifest {
            root: "app".to_string(),
            entries: pages.to_vec(),
            not_found: None,
        }
    }

    fn plan_manifest(
        planned: &[PlannedSource],
        pages: &[FrameworkEntry],
        apis: &[FrameworkEntry],
    ) -> Result<BuildManifest, String> {
        BuildManifest::plan(Path::new("/project"), planned, None, &app(pages), apis)
    }

    #[test]
    fn rejects_unsupported_plan_version() {
        let err = validate_plans(&[planned("app/x.ds", plan(3, vec![]))]).expect_err("v3 rejected");
        assert!(err.contains("unsupported dsc build plan version 3"), "{err}");
        assert!(err.contains("1, 2"), "{err}");
        let err = validate_plans(&[planned("app/x.ds", plan(0, vec![]))]).expect_err("v0 rejected");
        assert!(err.contains("unsupported"), "{err}");
    }

    #[test]
    fn rejects_malformed_and_duplicate_slots() {
        let mut bad = slot("a", "x", "app/page.ds", serde_json::json!({}));
        bad.entry.clear();
        assert!(validate_plans(&[planned("app/page.ds", plan(2, vec![bad]))]).is_err());

        // Duplicate ids across different files' plans are still duplicates.
        let dup = vec![
            planned(
                "app/a.ds",
                plan(2, vec![slot("a", "x", "app/a.ds", serde_json::json!({}))]),
            ),
            planned(
                "app/b.ds",
                plan(2, vec![slot("a", "y", "app/b.ds", serde_json::json!({}))]),
            ),
        ];
        let err = validate_plans(&dup).expect_err("duplicate id rejected");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn static_and_api_routes_plan_without_instances() {
        let manifest = plan_manifest(
            &[],
            &[page("/", "app/page.dsx"), page("/about", "app/about/page.ds")],
            &[api("/api/hello", "api/hello.ds")],
        )
        .expect("plan builds");
        let table = manifest.render_route_table();
        assert!(table.contains("○ / "), "{table}");
        assert!(table.contains("○ /about"), "{table}");
        assert!(table.contains("λ /api/hello"), "{table}");
        assert!(!table.contains('●'), "{table}");
        assert!(!table.contains('ƒ'), "{table}");
    }

    #[test]
    fn dynamic_route_without_disposition_fails() {
        let err = plan_manifest(&[], &[page("/posts/[slug]", "app/posts/page.ds")], &[])
            .expect_err("no disposition");
        assert!(
            err.contains("neither staticParams nor prerender = false"),
            "{err}"
        );
    }

    #[test]
    fn dynamic_route_with_v1_plan_fails_with_upgrade_hint() {
        // A v1 plan carries no disposition, so a bracket route cannot be
        // classified — fail rather than guess.
        let err = plan_manifest(
            &[planned("app/posts/page.ds", plan(1, vec![]))],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect_err("v1 plan has no disposition");
        assert!(err.contains("neither staticParams nor prerender = false"), "{err}");
    }

    #[test]
    fn prerender_false_marks_request_time() {
        let mut p = plan(2, vec![]);
        p.prerender = Some(false);
        let manifest = plan_manifest(
            &[planned("app/dashboard/page.ds", p)],
            &[page("/dashboard", "app/dashboard/page.ds")],
            &[],
        )
        .expect("plan builds");
        let table = manifest.render_route_table();
        assert!(table.contains("ƒ /dashboard"), "{table}");
        assert!(table.contains("prerender = false"), "{table}");
    }

    #[test]
    fn static_params_on_static_page_fails() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/page.dsx",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let err = plan_manifest(
            &[planned("app/page.dsx", p)],
            &[page("/", "app/page.dsx")],
            &[],
        )
        .expect_err("staticParams without bracket");
        assert!(err.contains("no [parameter]"), "{err}");
    }

    #[test]
    fn descriptor_mismatches_fail_by_name() {
        // Non-struct element.
        let p = plan(
            2,
            vec![slot("s1", "staticParams", "app/posts/page.ds", serde_json::json!({"node":"array","elem":{"node":"leaf","kind":"string"}}))],
        );
        let err = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect_err("non-struct element");
        assert!(err.contains("must be structs"), "{err}");

        // Field name mismatch.
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("id", "string")]),
            )],
        );
        let err = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect_err("field mismatch");
        assert!(err.contains("do not match the route parameters"), "{err}");
        assert!(err.contains("slug"), "{err}");

        // Non-string field.
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "number")]),
            )],
        );
        let err = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect_err("non-string field");
        assert!(err.contains("must be a string"), "{err}");
    }

    #[test]
    fn static_params_and_prerender_conflict() {
        let mut p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        p.prerender = Some(false);
        let err = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect_err("conflicting dispositions");
        assert!(err.contains("both staticParams"), "{err}");
    }

    #[test]
    fn static_params_expand_into_instances() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifest = plan_manifest(
            &[planned("/project/app/posts/page.ds", p)],
            &[page("/posts/[slug]", "/project/app/posts/page.ds")],
            &[],
        )
        .expect("plan builds");
        // Absolute vs project-relative file spellings must pair up.
        let values = BTreeMap::from([(
            "s1".to_string(),
            serde_json::json!([{"slug": "hello"}, {"slug": "world"}]),
        )]);
        manifest.expand_static_params(&values).expect("expands");
        let table = manifest.render_route_table();
        assert!(table.contains("● /posts/hello"), "{table}");
        assert!(table.contains("● /posts/world"), "{table}");
        assert!(table.contains("static: staticParams"), "{table}");
    }

    #[test]
    fn instance_collisions_fail_naming_both_routes() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifest = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[
                page("/posts/[slug]", "app/posts/page.ds"),
                page("/posts/hello", "app/posts/hello/page.ds"),
            ],
            &[],
        )
        .expect("plan builds");
        let values = BTreeMap::from([(
            "s1".to_string(),
            serde_json::json!([{"slug": "hello"}]),
        )]);
        let err = manifest.expand_static_params(&values).expect_err("collision");
        assert!(err.contains("/posts/hello"), "{err}");
        assert!(err.contains("collides"), "{err}");
    }

    #[test]
    fn duplicate_slug_within_params_fails() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifest = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect("plan builds");
        let values = BTreeMap::from([(
            "s1".to_string(),
            serde_json::json!([{"slug": "same"}, {"slug": "same"}]),
        )]);
        let err = manifest.expand_static_params(&values).expect_err("duplicate");
        assert!(err.contains("appears twice"), "{err}");
    }

    #[test]
    fn unsafe_param_values_fail() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifest = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect("plan builds");
        for raw in ["a/b", "a\\b", ""] {
            let values =
                BTreeMap::from([("s1".to_string(), serde_json::json!([{"slug": raw}]))]);
            assert!(
                manifest.expand_static_params(&values).is_err(),
                "value `{raw}` must be rejected"
            );
        }
    }

    #[test]
    fn missing_materialized_value_fails_before_render() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifest = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect("plan builds");
        let err = manifest
            .expand_static_params(&BTreeMap::new())
            .expect_err("no value");
        assert!(err.contains("was not materialized"), "{err}");
    }

    #[test]
    fn duplicate_page_templates_fail() {
        let err = plan_manifest(
            &[],
            &[
                page("/about", "app/about/page.ds"),
                page("/about", "app/about/page.dsx"),
            ],
            &[],
        )
        .expect_err("same template twice");
        assert!(err.contains("two app pages"), "{err}");
    }

    #[test]
    fn manifest_is_deterministic_across_repeated_plans() {
        let pages = vec![
            page("/", "app/page.dsx"),
            page("/posts/[slug]", "app/posts/page.ds"),
            page("/about", "app/about/page.ds"),
        ];
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let mut manifests = Vec::new();
        for _ in 0..3 {
            let mut m = plan_manifest(
                &[planned("app/posts/page.ds", p.clone())],
                &pages,
                &[api("/api/x", "api/x.ds")],
            )
            .expect("plan builds");
            m.expand_static_params(&BTreeMap::from([(
                "s1".to_string(),
                serde_json::json!([{"slug": "hello"}]),
            )]))
            .expect("expands");
            manifests.push(m.canonical_json().expect("json"));
        }
        assert!(
            manifests.windows(2).all(|pair| pair[0] == pair[1]),
            "identical inputs must produce identical manifest bytes"
        );
    }

    #[test]
    fn record_artifacts_hashes_every_file_sorted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let staged = temp.path();
        std::fs::create_dir_all(staged.join("client/posts")).expect("mkdir");
        std::fs::write(staged.join("client/index.html"), b"<html/>").expect("write");
        std::fs::write(staged.join("client/posts/index.html"), b"post").expect("write");
        std::fs::write(staged.join("_redirects"), b"/* / 301").expect("write");

        let mut manifest = plan_manifest(&[], &[page("/", "app/page.dsx")], &[])
            .expect("plan builds");
        manifest.record_artifacts(staged).expect("records");
        assert_eq!(manifest.artifacts.len(), 3);
        let paths: Vec<&str> = manifest.artifacts.iter().map(|a| a.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "artifacts are sorted by path");
        let html = std::fs::read(staged.join("client/index.html")).expect("read");
        assert_eq!(
            manifest.artifacts.iter().find(|a| a.path == "client/index.html").expect("row").digest,
            sha256_hex(&html)
        );
        // Directory entries and the manifest's own cache file never live
        // under staged dist, but an empty tree must record no artifacts.
        let mut empty = plan_manifest(&[], &[page("/", "app/page.dsx")], &[])
            .expect("plan builds");
        empty.record_artifacts(&staged.join("absent")).expect("records");
        assert!(empty.artifacts.is_empty());
    }

    #[test]
    fn project_relative_normalizes_spellings() {
        let root = Path::new("/project");
        assert_eq!(project_relative(root, "/project/app/page.ds"), "app/page.ds");
        assert_eq!(project_relative(root, "app/page.ds"), "app/page.ds");
        assert_eq!(project_relative(root, "./app/page.ds"), "app/page.ds");
        assert_eq!(
            project_relative(root, "\\project\\app\\page.ds"),
            "/project/app/page.ds"
        );
    }

    #[test]
    fn slot_records_carry_identity() {
        let p = plan(
            2,
            vec![slot(
                "s1",
                "staticParams",
                "app/posts/page.ds",
                params_descriptor(&[("slug", "string")]),
            )],
        );
        let descriptor = p.slots[0].descriptor.to_string();
        let manifest = plan_manifest(
            &[planned("app/posts/page.ds", p)],
            &[page("/posts/[slug]", "app/posts/page.ds")],
            &[],
        )
        .expect("plan builds");
        assert_eq!(manifest.slots.len(), 1);
        let slot = &manifest.slots[0];
        assert_eq!(slot.value_module, "deka:dev/s1");
        let expected = sha256_hex(descriptor.as_bytes());
        assert_eq!(slot.descriptor_digest, expected);
        assert_eq!(manifest.compiler.plan_version, 2);
    }
}
