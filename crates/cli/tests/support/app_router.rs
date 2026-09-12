//! Explicit source app-router fixture for surviving build/codegen tests.
//! The default `deka init` project is static while framework rendering is paused.

use std::{fs, path::Path};

pub fn init_project(dir: &Path) {
    for directory in ["app", "api", "src", "public"] {
        fs::create_dir_all(dir.join(directory)).unwrap();
    }
    for (file, source) in [
        (
            "deka.json",
            r#"{"name":"app-router-fixture","type":"serve","serve":{"mode":"ds"},"security":{"allow":{},"deny":{},"prompt":true}}"#,
        ),
        ("deka.lock", r#"{"lockfileVersion":1,"packages":{}}"#),
        ("index.html", default_index_html()),
        ("app/page.dsx", default_app_page_dsx()),
        ("app/layout.dsx", default_app_layout_dsx()),
        ("app/not-found.dsx", default_not_found_dsx()),
        ("public/style.css", default_public_style_css()),
    ] {
        fs::write(dir.join(file), source).unwrap();
    }
}

fn default_app_page_dsx() -> &'static str {
    "export fn Page() {\n  return <section><h1>Deka App</h1><p>Project initialized.</p></section>\n}\n"
}

fn default_app_layout_dsx() -> &'static str {
    "interface LayoutProps {\n  children: Component;\n}\nexport fn Layout(props: LayoutProps) {\n  return <main>{props.children}</main>\n}\n"
}

fn default_not_found_dsx() -> &'static str {
    "export fn Page() {\n  return <section><h1>Not found</h1></section>\n}\n"
}

fn default_index_html() -> &'static str {
    "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>Deka</title>\n    <link rel=\"stylesheet\" href=\"/style.css\" />\n    <!--deka-head-->\n  </head>\n  <body>\n    <div id=\"app\"><!--deka-app--></div>\n    <!--deka-scripts-->\n  </body>\n</html>\n"
}

fn default_public_style_css() -> &'static str {
    "body { font-family: system-ui, sans-serif; margin: 2rem; }\n"
}
