//! deka#738 F7: `dist/` must be deployable — no dev-scheme specifiers, and a
//! deployment carrying the project without `.cache/` serves every
//! build{}-backed route.
//!
//! Before the fix, emitted app JS kept `import ... from "deka:dev/<id>"`
//! verbatim while the backing module lived only at
//! `.cache/dekascript/build-values/<id>.js`. Copying the project anywhere
//! without `.cache/` and running `deka serve` 500'd on build-backed routes.
//!
//! The test MUST serve from the copy, not the project directory: serving in
//! place cannot distinguish "resolves because correct" from "resolves because
//! `.cache/` is adjacent" — that is exactly how the bug was missed.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Same lookup the build fixture harness uses: `DEKA_DSC`, then `dsc` on
/// PATH. Both `deka build` and the serve-time isolate compile exec dsc.
fn real_dsc() -> PathBuf {
    if let Ok(path) = std::env::var("DEKA_DSC") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let output = Command::new("which")
        .arg("dsc")
        .output()
        .expect("run which dsc");
    assert!(
        output.status.success(),
        "dist deploy test requires dsc (set DEKA_DSC or put dsc on PATH)"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

/// A build{} binding consumed at REQUEST time (`posts[0].title` renders into
/// the page), so the served module graph must resolve the build-value module
/// on every hit — a staticParams-only binding would be tree-shaken away by
/// the serve-time compile and never exercise resolution. `prerender = false`
/// keeps the route dynamic; there is no prerendered HTML to hide behind.
fn write_project(dir: &Path) {
    fs::create_dir_all(dir.join("app").join("posts").join("[slug]")).expect("mkdir page");
    fs::create_dir_all(dir.join("public")).expect("mkdir public");
    fs::write(
        dir.join("deka.json"),
        concat!(
            r#"{"name":"dist-deploy","type":"serve","#,
            r#""serve":{"mode":"ds"},"#,
            r#""security":{"allow":{},"deny":{},"prompt":true}}"#
        ),
    )
    .expect("write deka.json");
    fs::write(
        dir.join("deka.lock"),
        r#"{"lockfileVersion":1,"packages":{}}"#,
    )
    .expect("write deka.lock");
    fs::write(
        dir.join("index.html"),
        "<!doctype html>\n<html><head><title>dist-deploy</title></head>\
         <body><div id=\"app\"></div></body></html>\n",
    )
    .expect("write index.html");
    fs::write(
        dir.join("app").join("layout.dsx"),
        "interface LayoutProps {\n  children: Component;\n}\n\
         export fn Layout(props: LayoutProps) {\n  return <main>{props.children}</main>\n}\n",
    )
    .expect("write layout");
    fs::write(
        dir.join("app").join("page.dsx"),
        "export fn Page() {\n  return <section><h1>home</h1></section>\n}\n",
    )
    .expect("write page");
    fs::write(
        dir.join("app").join("posts").join("[slug]").join("page.dsx"),
        concat!(
            "interface PageProps { slug: string }\n",
            "struct Post { title: string }\n",
            "export const prerender = false\n",
            "const posts: Array<Post> = build {\n",
            "    return Ok([Post{title:\"hello from build\"}])\n",
            "}\n",
            "export fn Page(props: PageProps) {\n",
            "    return <article><h1>{posts[0].title}</h1><p>{props.slug}</p></article>;\n",
            "}\n",
        ),
    )
    .expect("write build page");
}

fn run_build(project: &Path, dsc: &Path) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project)
        .env("DEKA_DSC", dsc)
        .output()
        .expect("run deka build");
    assert!(
        output.status.success(),
        "deka build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every file under `dir`, relative forward-slash paths.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// The deployment contract: build output carries no dev-scheme specifier.
fn assert_dist_self_contained(dist: &Path) {
    let mut offenders = Vec::new();
    for file in walk_files(dist) {
        let bytes = fs::read(&file).expect("read dist file");
        if bytes
            .windows(b"deka:dev/".len())
            .any(|window| window == b"deka:dev/")
        {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "dist/ must not reference deka:dev/ specifiers: {offenders:?}"
    );
}

struct Serve {
    child: Child,
    log: PathBuf,
}

impl Serve {
    fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Copy the project into a fresh tree WITHOUT `.cache/` — the deployment
/// unit. Serving the copy (never the original) is what proves the build
/// value resolves from dist rather than from an adjacent cache.
fn copy_without_cache(project: &Path, copy: &Path) {
    fn copy_dir(src: &Path, dst: &Path) {
        for entry in fs::read_dir(src).expect("read_dir").flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".cache" || name.to_string_lossy().starts_with("dist.prev-") {
                continue;
            }
            let target = dst.join(&name);
            if path.is_dir() {
                copy_dir(&path, &target);
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).expect("mkdir");
                }
                fs::copy(&path, &target).expect("copy");
            }
        }
    }
    fs::create_dir_all(copy).expect("mkdir copy");
    copy_dir(project, copy);
}

#[test]
fn dist_deploy_serves_build_backed_route_without_cache() {
    let dsc = real_dsc();
    let project = tempfile::tempdir().expect("tempdir");
    write_project(project.path());
    run_build(project.path(), &dsc);

    let dist = project.path().join("dist");
    let build_values = dist.join("app").join(".build-values");
    assert!(
        build_values.is_dir(),
        "deka build must ship materialized build-value modules in dist/app/.build-values"
    );
    let shipped: Vec<String> = fs::read_dir(&build_values)
        .expect("read .build-values")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(shipped.len(), 1, "exactly one slot module ships: {shipped:?}");
    assert_dist_self_contained(&dist);

    let copy = tempfile::tempdir().expect("tempdir");
    copy_without_cache(project.path(), copy.path());
    assert!(
        !copy.path().join(".cache").exists(),
        "the deployment copy must not carry .cache/"
    );
    assert_dist_self_contained(&copy.path().join("dist"));

    let port = free_port();
    let log_path = copy.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("serve.log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(copy.path())
        .env("DEKA_DSC", &dsc)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let serve = Serve {
        child,
        log: log_path,
    };

    let http = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut status = String::new();
    let mut body = String::new();
    while Instant::now() < deadline {
        match http.get(format!("http://127.0.0.1:{port}/posts/hello")).send() {
            Ok(res) => {
                status = res.status().to_string();
                body = res.text().expect("read body");
                if status == "200 OK" {
                    break;
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(150)),
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    assert_eq!(
        status, "200 OK",
        "the build-backed route must serve 200 from a cache-less copy\nserve.log:\n{}",
        serve.log_text()
    );
    assert!(
        body.contains("hello from build"),
        "the response must carry the materialized build value, got:\n{body}\nserve.log:\n{}",
        serve.log_text()
    );
}
