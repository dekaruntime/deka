//! deka#971: a fresh, standalone project must go from `deka init` to a
//! resolved stdlib import through the real `deka install` -- against the
//! real registry+CDN request shape, not an in-process shortcut.
//!
//! This exercises the real topology: `deka init` scaffolds a project on
//! disk, a hermetic fixture server plays the deka.gg registry + R2 CDN
//! shape on 127.0.0.1 (the same test-only `DEKA_PM_REGISTRY_URL` /
//! `DEKA_PM_STDLIB_CDN` injection points as deka#797's
//! `grant_table_plumbing.rs` -- never a configuration channel, deka#801),
//! and the real `cli` binary is spawned as a child process to install and
//! then resolve the import.
//!
//! Two real bugs are pinned here, both found while investigating #971:
//!
//! 1. Sequential explicit installs (`deka install pkg-a` then later
//!    `deka install pkg-b`, i.e. what `deka add` does one call at a time)
//!    used to overwrite `deka.lock` with only the most recent call's
//!    closure, dropping every previously-installed `@deka/*` package even
//!    though `ds_modules/` and `deka.json` still had it. A fresh project
//!    that installed its dependencies one at a time -- an entirely normal
//!    way to add them -- ended up with a lockfile that satisfied nothing
//!    it should have.
//! 2. `deka.json` dependency versions written with the conventional
//!    `^`/`~`/`>=` semver-range prefix were passed to the registry
//!    literally (e.g. `@deka/greet@^1.0.0`), which cannot match any
//!    published release, so a bare `deka install` failed outright on any
//!    manifest using ordinary range syntax.
//!
//! `deka install`'s single-package explicit-add tests already exist in
//! `crates/pm/src/install.rs`; this file is the missing real-topology
//! proof: real scaffold, real child-process CLI, real module resolution
//! through `deka check` -- not a unit test of the resolver in isolation.
//! It intentionally does not touch the live deka.gg registry (per #933,
//! that dependency is already flaky elsewhere).

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::{Json, extract::Path as AxumPath, extract::State, routing::get};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// `deka check` compiles DekaScript/TSX through the pinned dsc; skip the
/// resolution-proof assertions (never the install assertions) when no
/// compiler is available in this environment.
fn dsc_available() -> Option<PathBuf> {
    if std::env::var_os("DEKA_NO_DSC").is_some() {
        return None;
    }
    if let Ok(path) = std::env::var("DEKA_DSC")
        && Path::new(&path).is_file()
    {
        return Some(PathBuf::from(path));
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/dsc");
    if local.is_file() {
        return Some(local);
    }
    None
}

/// Pack a one-file fixture stdlib package the way the real release workflow
/// does (`tar -czf` of the tree contents), with a real, callable export.
fn pack_fixture_package(work: &Path, name: &str, version: &str, source: &str) -> Vec<u8> {
    let tree = work.join(format!("{name}-tree"));
    fs::create_dir_all(&tree).expect("mkdir fixture tree");
    fs::write(
        tree.join("deka.json"),
        serde_json::json!({ "name": format!("@deka/{name}"), "version": version }).to_string(),
    )
    .expect("write fixture manifest");
    fs::write(tree.join("index.ds"), source).expect("write fixture module");

    let tarball = work.join(format!("{name}.tgz"));
    let status = Command::new("tar")
        .args([
            "-czf",
            tarball.to_str().expect("tarball path"),
            "-C",
            tree.to_str().expect("tree path"),
            ".",
        ])
        .status()
        .expect("run tar");
    assert!(status.success(), "tar must succeed");
    fs::read(&tarball).expect("read fixture tarball")
}

#[derive(Clone)]
struct FixtureState(Arc<std::collections::BTreeMap<&'static str, (Vec<&'static str>, Vec<u8>)>>);

async fn registry_json(
    AxumPath(name): AxumPath<String>,
    State(state): State<FixtureState>,
) -> Json<serde_json::Value> {
    let name = name.trim_end_matches(".json");
    let versions: Vec<&str> = state
        .0
        .get(name)
        .map(|(versions, _)| versions.clone())
        .unwrap_or_else(|| vec!["0.0.0-missing"]);
    Json(serde_json::json!({
        "name": name,
        "description": "install_ds_modules_e2e fixture",
        "versions": versions,
    }))
}

async fn tarball_bytes(
    AxumPath((name, _version, _file)): AxumPath<(String, String, String)>,
    State(state): State<FixtureState>,
) -> Vec<u8> {
    state
        .0
        .get(name.as_str())
        .map(|(_, bytes)| bytes.clone())
        .unwrap_or_default()
}

/// The fixture registry + CDN, alive for as long as this value is held.
struct FixtureRegistry {
    address: SocketAddr,
    _runtime: tokio::runtime::Runtime,
}

impl FixtureRegistry {
    fn start(
        packages: std::collections::BTreeMap<&'static str, (Vec<&'static str>, Vec<u8>)>,
    ) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let state = FixtureState(Arc::new(packages));
        let address = runtime.block_on(async {
            let app = axum::Router::new()
                .route("/api/registry/:name", get(registry_json))
                .route("/:name/:version/:file", get(tarball_bytes))
                .with_state(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture registry listener");
            let address = listener.local_addr().expect("fixture registry address");
            tokio::spawn(async move {
                axum::serve(listener, app).await.expect("fixture registry");
            });
            address
        });
        Self {
            address,
            _runtime: runtime,
        }
    }
}

fn run_cli(project: &Path, args: &[&str], registry: SocketAddr) -> (bool, String) {
    let mut command = Command::new(cli_bin());
    command
        .args(args)
        .current_dir(project)
        .env("DEKA_PM_REGISTRY_URL", format!("http://{registry}"))
        .env("DEKA_PM_STDLIB_CDN", format!("http://{registry}"))
        .env_remove("DEKA_HOST_GRANTS");
    if let Some(dsc) = dsc_available() {
        command.env("DEKA_DSC", dsc);
    }
    let output = command.output().expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn lock_package_names(project: &Path) -> Vec<String> {
    let lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.join("deka.lock")).expect("read lock"))
            .expect("parse lock");
    let mut names: Vec<String> = lock["packages"]
        .as_object()
        .expect("packages object")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

fn count_root_backups(project: &Path) -> usize {
    let Ok(entries) = fs::read_dir(project) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains("-backup-"))
        .count()
}

fn count_cache_entries(project: &Path) -> usize {
    let Ok(entries) = fs::read_dir(project.join(".cache")) else {
        return 0;
    };
    entries.count()
}

/// A fresh project scaffolded with the real `deka init`, then walked through
/// `deka install` exactly as a new user following the docs would: add one
/// stdlib package, then add a second one in a separate call, then import
/// and resolve one of them from `app/page.dsx`.
#[test]
fn fresh_scaffold_installs_two_packages_and_resolves_the_import() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "greet",
        (
            vec!["1.0.0"],
            pack_fixture_package(
                work.path(),
                "greet",
                "1.0.0",
                "export fn greet() string {\n  return \"hello from @deka/greet\"\n}\n",
            ),
        ),
    );
    packages.insert(
        "shout",
        (
            vec!["2.0.0"],
            pack_fixture_package(
                work.path(),
                "shout",
                "2.0.0",
                "export fn shout() string {\n  return \"SHOUT\"\n}\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");

    // 1. Real scaffold, exactly what a new user runs first.
    let (success, output) = run_cli(&project, &["init"], registry.address);
    assert!(success, "deka init must succeed: {output}");
    assert!(
        project.join("app/page.dsx").is_file(),
        "init must scaffold app/page.dsx"
    );

    // 2. Add the first package (`deka add`'s real one-call-per-package
    // shape, which is exactly how deka#971 manifested: nobody declares
    // every dependency by hand in one deka.json edit).
    let (success, output) = run_cli(&project, &["install", "greet"], registry.address);
    assert!(success, "deka install greet must succeed: {output}");
    assert_eq!(
        lock_package_names(&project),
        vec!["@deka/greet".to_string()]
    );

    // 3. Add a second package in a *separate* call. Before the deka#971
    // fix this truncated deka.lock down to only @deka/shout, discarding
    // @deka/greet even though it was still on disk and still declared in
    // deka.json.
    let (success, output) = run_cli(&project, &["install", "shout"], registry.address);
    assert!(success, "deka install shout must succeed: {output}");
    assert_eq!(
        lock_package_names(&project),
        vec!["@deka/greet".to_string(), "@deka/shout".to_string()],
        "the second explicit install must not drop the first package's lock entry (deka#971)"
    );
    assert!(
        project.join("ds_modules/@deka/greet/index.ds").is_file(),
        "@deka/greet must still be physically installed"
    );
    assert!(
        project.join("ds_modules/@deka/shout/index.ds").is_file(),
        "@deka/shout must be installed"
    );

    // A third, bare `deka install` (the sync-from-manifest form) must be
    // stable: re-running it must not lose anything either.
    let (success, output) = run_cli(&project, &["install"], registry.address);
    assert!(success, "bare deka install must succeed: {output}");
    assert_eq!(
        lock_package_names(&project),
        vec!["@deka/greet".to_string(), "@deka/shout".to_string()]
    );

    // 4. Import one of the now-installed packages from the real scaffold
    // entry point, the way the tracker's user did.
    let page = project.join("app/page.dsx");
    let original = fs::read_to_string(&page).expect("read page.dsx");
    assert!(
        original.contains("export fn Page()"),
        "scaffold's page.dsx must still have the shape this rewrite assumes"
    );
    let rewritten = format!(
        "import {{ greet }} from \"@deka/greet\"\n\n{}",
        original.replacen(
            "<h1>Deka App</h1>",
            "<h1>{greet()}</h1>",
            1,
        )
    );
    fs::write(&page, rewritten).expect("write page.dsx with stdlib import");

    let Some(_dsc) = dsc_available() else {
        println!("SKIP resolution-proof assertion: dsc unavailable");
        return;
    };

    // 5. The actual claim of #971: the import resolves. `deka check` runs
    // the real module graph + type check against the real ds_modules/ tree
    // `deka install` just populated -- no dev server needed to prove
    // resolution.
    let (success, output) = run_cli(&project, &["check", "app/page.dsx"], registry.address);
    assert!(
        success,
        "stdlib import from a freshly-installed package must resolve: {output}"
    );
}

/// deka#971 side finding: a `deka.json` dependency written with the
/// conventional `^`/`~` semver-range prefix must still resolve, not be
/// passed to the registry as a literal (nonexistent) version string.
#[test]
fn caret_and_tilde_ranges_in_manifest_resolve_through_bare_install() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "greet",
        (
            vec!["1.2.3"],
            pack_fixture_package(
                work.path(),
                "greet",
                "1.2.3",
                "export fn greet() string { return \"hi\" }\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");
    fs::write(
        project.join("deka.json"),
        serde_json::json!({
            "name": "range-e2e",
            "dependencies": { "@deka/greet": "^1.2.3" }
        })
        .to_string(),
    )
    .expect("write manifest");

    let (success, output) = run_cli(&project, &["install"], registry.address);
    assert!(
        success,
        "a `^`-range dependency must resolve to its base version, not fail as a literal spec: {output}"
    );
    assert_eq!(
        lock_package_names(&project),
        vec!["@deka/greet".to_string()]
    );
    let lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.join("deka.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["packages"]["@deka/greet"][0].as_str(),
        Some("@deka/greet@1.2.3"),
        "the locked version must be the stripped base version, not the literal range string"
    );
}

/// deka#1011: the actual bug was in the RESOLUTION PATH, not in a pure
/// comparison helper -- `select_version` used to match the requested
/// string literally against the registry, so `^0.3.1` only ever "worked"
/// when the registry happened to publish exactly `0.3.1` and nothing
/// higher. This drives a registry that publishes several 0.3.x/0.4.x
/// releases through the real `deka install` child process and asserts the
/// HIGHEST compatible one is what actually gets locked -- a test against
/// the pure comparison function in isolation would not catch a bug in how
/// `install_from_registry` wires the requested range into that function.
#[test]
fn caret_range_resolves_to_highest_published_compatible_version_through_real_install() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "greet",
        (
            vec!["0.3.0", "0.3.1", "0.3.4", "0.4.0"],
            pack_fixture_package(
                work.path(),
                "greet",
                "0.3.4",
                "export fn greet() string { return \"hi\" }\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");
    fs::write(
        project.join("deka.json"),
        serde_json::json!({
            "name": "range-highest-e2e",
            // ^0.3.1 must resolve to 0.3.4 (the highest 0.3.x release) --
            // never 0.3.1 literally, and never 0.4.0 (caret on a 0.x
            // version only floats the patch digit).
            "dependencies": { "@deka/greet": "^0.3.1" }
        })
        .to_string(),
    )
    .expect("write manifest");

    let (success, output) = run_cli(&project, &["install"], registry.address);
    assert!(success, "a `^`-range install against a real multi-version registry must succeed: {output}");

    let lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.join("deka.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["packages"]["@deka/greet"][0].as_str(),
        Some("@deka/greet@0.3.4"),
        "`^0.3.1` against a registry publishing 0.3.0/0.3.1/0.3.4/0.4.0 must lock the HIGHEST \
         compatible release (0.3.4), proving resolution runs through the real registry path"
    );
}

/// deka#1011: a range matching nothing published must fail with a clear
/// diagnostic naming the constraint and what IS available -- not a bare
/// "not found" -- and it must fail through the real install path, not just
/// the resolver called directly.
#[test]
fn range_matching_nothing_published_fails_with_a_clear_diagnostic() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "greet",
        (
            vec!["0.1.0", "0.2.0"],
            pack_fixture_package(
                work.path(),
                "greet",
                "0.2.0",
                "export fn greet() string { return \"hi\" }\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");
    fs::write(
        project.join("deka.json"),
        serde_json::json!({
            "name": "range-no-match-e2e",
            "dependencies": { "@deka/greet": "^1.0.0" }
        })
        .to_string(),
    )
    .expect("write manifest");

    let (success, output) = run_cli(&project, &["install"], registry.address);
    assert!(
        !success,
        "a range matching nothing published must fail, not silently succeed: {output}"
    );
    assert!(
        output.contains("^1.0.0"),
        "the diagnostic must name the unmet constraint: {output}"
    );
    assert!(
        output.contains("0.1.0") && output.contains("0.2.0"),
        "the diagnostic must list the versions that ARE available: {output}"
    );
}

#[test]
fn repeated_installs_do_not_accumulate_install_backups_or_cache_entries() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "greet",
        (
            vec!["1.0.0"],
            pack_fixture_package(
                work.path(),
                "greet",
                "1.0.0",
                "export fn greet() string { return \"hello\" }\n",
            ),
        ),
    );
    packages.insert(
        "shout",
        (
            vec!["2.0.0"],
            pack_fixture_package(
                work.path(),
                "shout",
                "2.0.0",
                "export fn shout() string { return \"shout\" }\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");
    let (success, output) = run_cli(&project, &["init"], registry.address);
    assert!(success, "deka init must succeed: {output}");

    let baseline_backups = count_root_backups(&project);
    let baseline_cache_files = count_cache_entries(&project);
    let operations = vec![
        vec!["install", "greet"],
        vec!["install", "shout"],
        vec!["install"],
    ];
    for args in operations {
        let (success, output) = run_cli(&project, &args, registry.address);
        assert!(success, "repeated install command should succeed: {output}");
        assert_eq!(
            count_root_backups(&project),
            baseline_backups,
            "install back-ups should not accumulate across repeated commands"
        );
        assert_eq!(
            count_cache_entries(&project),
            baseline_cache_files,
            ".cache entries should not accumulate across repeated commands"
        );
    }
}

/// deka#1012 / deka#1014: a failing `install` that runs after a successful
/// one must (a) restore `deka.lock` / `deka.json` / `deka.grants.json` to
/// exactly what they were before the failing call, byte for byte, and (b)
/// leave zero recovery-backup litter behind in either the project root or
/// `.cache` -- not just "no more than before", genuinely none. A normal
/// rollback fully consumes its own backups (restoring them into place and
/// removing the copy), so a failure report naming leftover backups is only
/// meaningful when rollback itself fails; that path is covered at the unit
/// level in `pm::recovery_report`, not by forcing a real rollback failure
/// through the CLI here.
#[test]
fn failed_install_restores_prior_state_and_leaves_no_backup_litter() {
    let work = tempfile::tempdir().expect("work dir");
    let mut packages = std::collections::BTreeMap::new();
    packages.insert(
        "fs",
        (
            vec!["1.0.0"],
            pack_fixture_package(
                work.path(),
                "fs",
                "1.0.0",
                "export fn fsOk() string { return \"fs\" }\n",
            ),
        ),
    );
    let registry = FixtureRegistry::start(packages);

    let project = work.path().join("app");
    fs::create_dir_all(&project).expect("mkdir project");
    let (success, output) = run_cli(&project, &["init"], registry.address);
    assert!(success, "deka init must succeed: {output}");

    let (success, output) = run_cli(&project, &["install", "fs"], registry.address);
    assert!(success, "initial install must succeed: {output}");
    assert!(project.join("deka.grants.json").is_file(), "grant table must be written");

    let lock_before = fs::read_to_string(project.join("deka.lock")).expect("lock before");
    let manifest_before = fs::read_to_string(project.join("deka.json")).expect("manifest before");
    let grants_before = fs::read_to_string(project.join("deka.grants.json")).expect("grants before");
    assert_eq!(count_root_backups(&project), 0, "clean before the failing call");
    assert_eq!(count_cache_entries(&project), 0, "clean before the failing call");

    // "does-not-exist" maps to @deka/does-not-exist (bare names map to
    // @deka/*) and the fixture registry serves no such package, so the
    // installer resolves an empty/source-less artifact and rejects it --
    // after "fs" (already locked) has run through the same per-package loop
    // and opened a transaction ahead of it.
    let (success, output) = run_cli(&project, &["install", "fs", "does-not-exist"], registry.address);
    assert!(
        !success,
        "install with a missing package should fail after a partial successful install: {output}"
    );
    assert!(
        output.contains("does-not-exist"),
        "failure should name the offending package: {output}"
    );

    assert_eq!(
        fs::read_to_string(project.join("deka.lock")).expect("lock after"),
        lock_before,
        "deka.lock must be restored to its exact pre-failure content"
    );
    assert_eq!(
        fs::read_to_string(project.join("deka.json")).expect("manifest after"),
        manifest_before,
        "deka.json must be restored to its exact pre-failure content"
    );
    assert_eq!(
        fs::read_to_string(project.join("deka.grants.json")).expect("grants after"),
        grants_before,
        "deka.grants.json must be restored to its exact pre-failure content"
    );
    assert_eq!(
        count_root_backups(&project),
        0,
        "no backup litter in the project root after a successful rollback"
    );
    assert_eq!(
        count_cache_entries(&project),
        0,
        "no backup litter left in .cache after a successful rollback"
    );
}
