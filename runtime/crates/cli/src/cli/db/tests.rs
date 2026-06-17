use super::generate::{
    annotate_untyped_params, extract_struct_models, generate_db_artifacts, map_sql_type,
    render_client_phpx, render_generated_schema_json, resolve_generate_input, resolve_model_entry,
    to_table_name,
};
use migrate::{persist_migration_state, render_init_migration};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use std::fs;
use std::path::Path;

#[test]
fn rejects_missing_path() {
    let err = resolve_model_entry(Path::new("missing/thing.phpx"), "missing/thing.phpx")
        .expect_err("expected missing path to fail");
    assert!(err.contains("model input not found"));
}

#[test]
fn resolves_project_alias_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let types_dir = dir.path().join("types");
    fs::create_dir_all(&types_dir).expect("create types");
    let model = types_dir.join("index.phpx");
    fs::write(&model, "struct User { $id: int @id }").expect("write model");

    let input = "@/types".to_string();
    let resolved = resolve_generate_input(dir.path(), Some(&input)).expect("resolve");
    assert_eq!(resolved, model);
}

#[test]
fn extracts_models_and_fields() {
    let source = r#"
struct User {
  $id: Option<int> @id
  $email: string
}

struct Package {
  $name: string
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "User");
    assert_eq!(models[0].fields.len(), 2);
    assert_eq!(models[0].fields[0].name, "id");
    assert_eq!(models[0].fields[0].ty, "Option<int>");
    assert_eq!(models[0].fields[0].annotations.len(), 1);
    assert_eq!(models[0].fields[0].annotations[0].name, "id");
    assert_eq!(models[1].name, "Package");
    assert_eq!(models[1].fields.len(), 1);
}

#[test]
fn migration_respects_field_annotations() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @unique @map("email_address")
  $age: Option<int> @default(18)
  $name: string @index("users_name_idx")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let migration = render_init_migration(&models);
    assert!(migration.contains("\"id\" BIGSERIAL NOT NULL PRIMARY KEY"));
    assert!(migration.contains("\"email_address\" TEXT NOT NULL UNIQUE"));
    assert!(migration.contains("\"age\" BIGINT DEFAULT 18"));
    assert!(migration.contains("CREATE INDEX IF NOT EXISTS \"users_name_idx\""));
}

#[test]
fn migration_relation_field_is_virtual_and_belongsto_fk_is_indexed() {
    let source = r#"
struct Post {
  $id: int @id @autoIncrement
  $authorId: int
  $author: User @relation("belongsTo", "User", "authorId")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let migration = render_init_migration(&models);
    assert!(migration.contains("\"authorId\" BIGINT NOT NULL"));
    assert!(!migration.contains("\"author\" TEXT"));
    assert!(migration.contains("CREATE INDEX IF NOT EXISTS \"idx_posts_authorId\""));
}

#[test]
fn migration_relation_belongsto_fk_uses_mapped_column_name_for_index() {
    let source = r#"
struct Post {
  $id: int @id @autoIncrement
  $authorId: int @map("author_id")
  $author: User @relation("belongsTo", "User", "authorId")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let migration = render_init_migration(&models);
    assert!(migration.contains("\"author_id\" BIGINT NOT NULL"));
    assert!(migration.contains("CREATE INDEX IF NOT EXISTS \"idx_posts_author_id\""));
}

#[test]
fn migration_relation_belongsto_without_fk_field_does_not_emit_index() {
    let source = r#"
struct Post {
  $id: int @id @autoIncrement
  $author: User @relation("belongsTo", "User", "authorId")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let migration = render_init_migration(&models);
    assert!(!migration.contains("idx_posts_authorId"));
    assert!(!migration.contains("idx_posts_author_id"));
}

#[test]
fn generated_client_has_query_builder_api() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @unique
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let client = render_client_phpx(&models);
    assert!(client.contains("export function createClient"));
    assert!(client.contains("export function close"));
    assert!(client.contains("export function insert"));
    assert!(client.contains("export function select"));
    assert!(client.contains("export function update"));
    assert!(client.contains("export function deleteQuery"));
    assert!(client.contains("export function loadRelation"));
    assert!(client.contains("function field_value"));
    assert!(client.contains("if ($expr === null)"));
    assert!(
        client.contains("connect: fn($driver: mixed, $config: mixed) => connect($driver, $config)")
    );
    assert!(
        client.contains("withHandle: fn($nextHandle: mixed) => createClient($meta, $nextHandle)")
    );
    assert!(client.contains("transaction: fn($fn: mixed) => transaction($handle, $fn)"));
    assert!(client.contains("selectMany: fn($model: mixed, $where: mixed = null, $order: mixed = null, $limit: mixed = null, $offset: mixed = null, $includes: mixed = null) => selectMany($handle, $meta, $model, $where, $order, $limit, $offset, $includes)"));
    assert!(client.contains("insertOne: fn($model: mixed, $row: mixed, $returning: mixed = false) => insertOne($handle, $model, $row, $returning)"));
    assert!(client.contains("select: fn() => select($handle, $meta)"));
    assert!(client.contains("insert: fn($model: mixed) => insert($handle, $model)"));
    assert!(client.contains("update: fn($model: mixed) => update($handle, $model)"));
    assert!(client.contains("'delete': fn($model: mixed) => deleteQuery($handle, $model)"));
    assert!(client.contains("loadRelation: fn($model: mixed, $row: mixed, $field: mixed) => loadRelation($handle, $meta, $model, $row, $field)"));
    assert!(client.contains("ilike: fn($column: mixed, $value: mixed) => ilike($column, $value)"));
    assert!(client.contains("isNull: fn($column: mixed) => isNull($column)"));
    assert!(client.contains("asc: fn($column: mixed) => asc($column)"));
    assert!(client.contains("desc: fn($column: mixed) => desc($column)"));
    assert!(client.contains("limit: fn($value: mixed) => limit($value)"));
    assert!(client.contains("offset: fn($value: mixed) => offset($value)"));
}

#[test]
fn generated_client_load_relation_logic_covers_relation_kinds() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let client = render_client_phpx(&models);

    assert!(client.contains("export function loadRelation"));
    assert!(client.contains("if ($kind === 'hasMany')"));
    assert!(client.contains(
        "return selectMany($handle, $meta, $target, eq($fk, $row['id']), null, null, null, null)"
    ));
    assert!(client.contains("if ($kind === 'belongsTo' || $kind === 'hasOne')"));
    assert!(
        client.contains("return selectOne($handle, $meta, $target, eq('id', $row[$fk]), null)")
    );
    assert!(client.contains("return result_err('unknown relation')"));
    assert!(client.contains("return result_err('unsupported relation kind')"));
}

#[test]
fn generated_schema_json_contains_db_names() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @map("email_address")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let schema = render_generated_schema_json(&models);
    assert!(schema.contains("\"table\": \"users\""));
    assert!(schema.contains("\"db_name\": \"email_address\""));
}

#[test]
fn generated_schema_json_contains_relation_metadata() {
    let source = r#"
struct Post {
  $id: int @id @autoIncrement
  $authorId: int
  $author: User @relation("belongsTo", "User", "authorId")
}
"#;
    let models = extract_struct_models(source, "inline.phpx".to_string()).expect("models");
    let schema = render_generated_schema_json(&models);
    assert!(schema.contains("\"relations\""));
    assert!(schema.contains("\"kind\": \"belongsTo\""));
    assert!(schema.contains("\"foreignKey\": \"authorId\""));
}

#[test]
fn generate_db_artifacts_writes_expected_files() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @unique
}
"#;
    let models = extract_struct_models(source, "types/index.phpx".to_string()).expect("models");

    let dir = tempfile::tempdir().expect("tempdir");
    let source_path = dir.path().join("types").join("index.phpx");
    fs::create_dir_all(source_path.parent().expect("parent")).expect("mkdir");
    fs::write(&source_path, source).expect("write source");

    let generated = generate_db_artifacts(dir.path(), &source_path, &models).expect("generated");
    assert_eq!(generated, 6);
    assert!(dir.path().join("db/index.phpx").exists());
    assert!(dir.path().join("db/client.phpx").exists());
    assert!(dir.path().join("db/meta.phpx").exists());
    assert!(dir.path().join("db/_state.json").exists());
    assert!(dir.path().join("db/migrations/0001_init.sql").exists());
    assert!(dir.path().join("db/.generated/schema.json").exists());
}

#[test]
fn generated_index_phpx_is_parser_safe() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @unique
}
"#;
    let models = extract_struct_models(source, "types/index.phpx".to_string()).expect("models");

    let dir = tempfile::tempdir().expect("tempdir");
    let source_path = dir.path().join("types").join("index.phpx");
    fs::create_dir_all(source_path.parent().expect("parent")).expect("mkdir");
    fs::write(&source_path, source).expect("write source");
    generate_db_artifacts(dir.path(), &source_path, &models).expect("generated");

    let generated = fs::read_to_string(dir.path().join("db/index.phpx")).expect("read index");
    let generated = mask_module_syntax_for_parser(&generated);
    let arena = bumpalo::Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(generated.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "generated index parse errors: {:?}",
        program.errors
    );
}

#[test]
fn generated_client_phpx_is_parser_safe() {
    let source = r#"
struct User {
  $id: int @id @autoIncrement
  $email: string @unique
}
"#;
    let models = extract_struct_models(source, "types/index.phpx".to_string()).expect("models");

    let dir = tempfile::tempdir().expect("tempdir");
    let source_path = dir.path().join("types").join("index.phpx");
    fs::create_dir_all(source_path.parent().expect("parent")).expect("mkdir");
    fs::write(&source_path, source).expect("write source");
    generate_db_artifacts(dir.path(), &source_path, &models).expect("generated");

    let generated = fs::read_to_string(dir.path().join("db/client.phpx")).expect("read client");
    let generated = mask_module_syntax_for_parser(&generated);
    let arena = bumpalo::Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(generated.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "generated client parse errors: {:?}",
        program.errors
    );
}

#[test]
fn annotate_untyped_params_rewrites_function_and_fn_params() {
    let src = "function a($x, &$y = null) { return fn($z, ...$rest) => $z; }";
    let got = annotate_untyped_params(src);
    assert!(got.contains("function a($x: mixed, &$y: mixed = null)"));
    assert!(got.contains("fn($z: mixed, ...$rest: mixed)"));
}

#[test]
fn annotate_untyped_params_preserves_typed_params() {
    let src = "function a($x: int, $y: string = 'ok') { return fn($z: bool) => $z; }";
    let got = annotate_untyped_params(src);
    assert_eq!(got, src);
}

fn mask_module_syntax_for_parser(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for segment in source.split_inclusive('\n') {
        let trimmed = segment.trim();
        let masked = trimmed.starts_with("import ")
            || trimmed.starts_with("export {")
            || (trimmed.starts_with("export ") && !trimmed.starts_with("export function"));
        if masked {
            out.push_str(
                &segment
                    .chars()
                    .map(|ch| if ch == '\n' { '\n' } else { ' ' })
                    .collect::<String>(),
            );
            continue;
        }
        if trimmed.starts_with("export function") {
            if let Some(idx) = segment.find("export") {
                out.push_str(&segment[..idx]);
                out.push_str("      ");
                out.push_str(&segment[idx + 6..]);
                continue;
            }
        }
        out.push_str(segment);
    }
    out
}

#[test]
fn maps_option_types_to_nullable_sql() {
    let (ty, nullable) = map_sql_type("Option<int>");
    assert_eq!(ty, "BIGINT");
    assert!(nullable);
}

#[test]
fn maps_array_types_to_jsonb() {
    let (ty_plain, nullable_plain) = map_sql_type("array");
    assert_eq!(ty_plain, "JSONB");
    assert!(!nullable_plain);

    let (ty_applied, nullable_applied) = map_sql_type("array<string>");
    assert_eq!(ty_applied, "JSONB");
    assert!(!nullable_applied);
}

#[test]
fn table_name_is_snake_plural() {
    assert_eq!(to_table_name("User"), "users");
    assert_eq!(to_table_name("PackageVersion"), "package_versions");
}

#[test]
fn migration_state_is_persisted_to_state_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_dir = dir.path().join("db");
    fs::create_dir_all(&db_dir).expect("mkdir db");
    let state_path = db_dir.join("_state.json");
    fs::write(
        &state_path,
        r#"{
  "version": 1,
  "source": "types/index.phpx"
}"#,
    )
    .expect("write state");

    let applied = ["0001_init.sql".to_string(), "0002_users.sql".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    persist_migration_state(&db_dir, &applied, 1, 0).expect("persist");

    let raw = fs::read_to_string(&state_path).expect("read");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(value.get("version").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(
        value
            .get("migration_applied_total")
            .and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        value
            .get("migration_last_applied_count")
            .and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        value
            .get("migration_applied_versions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len()),
        Some(2)
    );
}
