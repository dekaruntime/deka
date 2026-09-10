use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempDir};

const CLI_SOURCE: &str = include_str!("ui/cli.ts");
const FULLSCREEN_SOURCE: &str = include_str!("ui/fullscreen.tsx");
const UI_SOURCE: &str = include_str!("ui/introspect-ui.tsx");
const INTROSPECT_SOURCE: &str = include_str!("ui/introspect.ts");

const FILES: &[(&str, &str)] = &[
    ("cli.ts", CLI_SOURCE),
    ("fullscreen.tsx", FULLSCREEN_SOURCE),
    ("introspect-ui.tsx", UI_SOURCE),
    ("introspect.ts", INTROSPECT_SOURCE),
];

pub struct UiAssets {
    _directory: TempDir,
    cli_path: PathBuf,
    ui_path: PathBuf,
}

impl UiAssets {
    pub fn materialize() -> Result<Self, String> {
        let directory = Builder::new()
            .prefix("deka-introspect-ui-")
            .tempdir()
            .map_err(|err| format!("failed to create introspect UI directory: {err}"))?;

        for (filename, source) in FILES {
            fs::write(directory.path().join(filename), source)
                .map_err(|err| format!("failed to materialize introspect UI {filename}: {err}"))?;
        }

        Ok(Self {
            cli_path: directory.path().join("cli.ts"),
            ui_path: directory.path().join("introspect-ui.tsx"),
            _directory: directory,
        })
    }

    pub fn cli_path(&self) -> &Path {
        &self.cli_path
    }

    pub fn ui_path(&self) -> &Path {
        &self.ui_path
    }
}
