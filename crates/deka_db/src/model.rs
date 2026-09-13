use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

#[derive(Debug, Clone)]
pub(super) struct ModelDef {
    pub(super) name: String,
    pub(super) fields: Vec<FieldDef>,
}

#[derive(Debug, Clone)]
pub(super) struct FieldDef {
    pub(super) name: String,
    pub(super) ty: String,
    pub(super) annotations: Vec<FieldAnnotationDef>,
}

#[derive(Debug, Clone)]
pub(super) struct FieldAnnotationDef {
    pub(super) name: String,
    pub(super) args: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RelationSpec {
    pub(super) kind: String,
    pub(super) _model: String,
    pub(super) foreign_key: String,
}

impl FieldDef {
    pub(super) fn annotation(&self, name: &str) -> Option<&FieldAnnotationDef> {
        self.annotations.iter().find(|ann| ann.name == name)
    }

    pub(super) fn has_annotation(&self, name: &str) -> bool {
        self.annotation(name).is_some()
    }

    pub(super) fn mapped_name(&self) -> String {
        self.annotation("map")
            .and_then(|ann| ann.args.first())
            .map(|arg| unquote(arg))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.name.clone())
    }

    pub(super) fn relation_spec(&self) -> Option<RelationSpec> {
        let ann = self.annotation("relation")?;
        if ann.args.len() != 3 {
            return None;
        }
        let kind = unquote(&ann.args[0]);
        let model = unquote(&ann.args[1]);
        let foreign_key = unquote(&ann.args[2]);
        if kind.is_empty() || model.is_empty() || foreign_key.is_empty() {
            return None;
        }
        if kind != "hasMany" && kind != "belongsTo" && kind != "hasOne" {
            return None;
        }
        Some(RelationSpec {
            kind,
            _model: model,
            foreign_key,
        })
    }
}

pub(super) fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'')
        {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}

pub(super) fn to_table_name(model_name: &str) -> String {
    let mut out = String::new();
    for (idx, ch) in model_name.chars().enumerate() {
        if ch.is_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    if out.ends_with('s') {
        out
    } else {
        format!("{}s", out)
    }
}

pub(super) fn map_sql_type(ty: &str) -> (&'static str, bool) {
    let trimmed = ty.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered == "array" || lowered.starts_with("array<") {
        return ("JSONB", false);
    }
    if let Some(inner) = trimmed
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let (mapped, _) = map_sql_type(inner);
        return (mapped, true);
    }
    match trimmed {
        "int" | "i64" | "u64" | "i32" | "u32" => ("BIGINT", false),
        "float" | "double" | "f64" | "f32" => ("DOUBLE PRECISION", false),
        "bool" | "boolean" => ("BOOLEAN", false),
        "string" | "String" => ("TEXT", false),
        _ => ("TEXT", false),
    }
}

pub(super) fn persist_migration_state(
    db_dir: &Path,
    applied_versions: &HashSet<String>,
    applied_now: usize,
    skipped: usize,
) -> Result<(), String> {
    let state_path = db_dir.join("_state.json");
    let mut state = if state_path.is_file() {
        let raw = fs::read_to_string(&state_path)
            .map_err(|err| format!("failed to read {}: {}", state_path.display(), err))?;
        serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !state.is_object() {
        state = json!({});
    }

    let mut versions = applied_versions.iter().cloned().collect::<Vec<_>>();
    versions.sort();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Some(obj) = state.as_object_mut() {
        obj.insert("migration_last_run_unix".to_string(), json!(now));
        obj.insert("migration_applied_total".to_string(), json!(versions.len()));
        obj.insert(
            "migration_last_applied_count".to_string(),
            json!(applied_now),
        );
        obj.insert("migration_last_skipped_count".to_string(), json!(skipped));
        obj.insert("migration_applied_versions".to_string(), json!(versions));
    }

    let rendered = serde_json::to_string_pretty(&state)
        .map_err(|err| format!("failed to render migration state json: {}", err))?;
    fs::write(&state_path, rendered)
        .map_err(|err| format!("failed to write {}: {}", state_path.display(), err))?;
    Ok(())
}

pub(super) fn render_init_migration(models: &[ModelDef]) -> String {
    let mut out = String::new();
    out.push_str("-- AUTO-GENERATED MIGRATION - DO NOT EDIT MANUALLY\n");
    out.push_str("-- Generated by deka db generate\n\n");
    for model in models {
        let table = to_table_name(&model.name);
        out.push_str(&format!("CREATE TABLE IF NOT EXISTS \"{}\" (\n", table));
        let mut defs: Vec<String> = Vec::new();
        let mut index_defs: Vec<String> = Vec::new();
        let mut fk_lookup: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for field in &model.fields {
            if field.relation_spec().is_some() {
                continue;
            }
            let mapped = field.mapped_name();
            fk_lookup.insert(field.name.clone(), mapped.clone());
            fk_lookup.insert(mapped.clone(), mapped);
        }
        let mut seen_indexes = std::collections::HashSet::new();
        for field in &model.fields {
            if let Some(relation) = field.relation_spec() {
                if relation.kind == "belongsTo" {
                    if let Some(db_fk) = fk_lookup.get(&relation.foreign_key) {
                        let index_name = format!("idx_{}_{}", table, db_fk);
                        let stmt = format!(
                            "CREATE INDEX IF NOT EXISTS \"{}\" ON \"{}\" (\"{}\");",
                            index_name, table, db_fk
                        );
                        if seen_indexes.insert(stmt.clone()) {
                            index_defs.push(stmt);
                        }
                    }
                }
                continue;
            }
            let (sql_ty, nullable) = map_sql_type(&field.ty);
            let db_name = field.mapped_name();
            let mut def = if field.has_annotation("autoIncrement") {
                format!("  \"{}\" BIGSERIAL", db_name)
            } else {
                format!("  \"{}\" {}", db_name, sql_ty)
            };
            if !nullable {
                def.push_str(" NOT NULL");
            }
            if field.has_annotation("id") || field.name == "id" {
                def.push_str(" PRIMARY KEY");
            }
            if field.has_annotation("unique") {
                def.push_str(" UNIQUE");
            }
            if let Some(default_ann) = field.annotation("default") {
                if let Some(raw) = default_ann.args.first() {
                    let literal = default_sql_literal(raw);
                    def.push_str(&format!(" DEFAULT {}", literal));
                }
            }
            defs.push(def);

            if let Some(index_ann) = field.annotation("index") {
                let explicit = index_ann.args.first().map(|arg| unquote(arg));
                let index_name = explicit
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("idx_{}_{}", table, db_name));
                let stmt = format!(
                    "CREATE INDEX IF NOT EXISTS \"{}\" ON \"{}\" (\"{}\");",
                    index_name, table, db_name
                );
                if seen_indexes.insert(stmt.clone()) {
                    index_defs.push(stmt);
                }
            }
        }
        out.push_str(&defs.join(",\n"));
        out.push_str("\n);\n\n");
        for idx in index_defs {
            out.push_str(&idx);
            out.push('\n');
        }
        if !model.fields.is_empty() {
            out.push('\n');
        }
    }
    out
}

fn default_sql_literal(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        "TRUE".to_string()
    } else if trimmed.eq_ignore_ascii_case("false") {
        "FALSE".to_string()
    } else if trimmed.eq_ignore_ascii_case("null") {
        "NULL".to_string()
    } else if looks_like_number(trimmed) {
        trimmed.to_string()
    } else {
        format!("'{}'", unquote(trimmed).replace('\'', "''"))
    }
}

fn looks_like_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut start = 0usize;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        start = 1;
    }
    if start >= bytes.len() {
        return false;
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    for &b in &bytes[start..] {
        if b.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if b == b'.' && !saw_dot {
            saw_dot = true;
            continue;
        }
        return false;
    }
    saw_digit
}
