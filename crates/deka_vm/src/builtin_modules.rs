//! One compiler table resolves Rust builtin modules before consumer packages.
use crate::{Hosts, Result};
use std::{collections::HashMap, path::PathBuf};
struct Module {
    name: &'static str,
    prefix: &'static str,
    direct: &'static [&'static str],
}
const MODULES: &[Module] = &[
    Module {
        name: "vm:host",
        prefix: "",
        direct: &[],
    },
    Module {
        name: "io",
        prefix: "io_",
        direct: &["echo", "print"],
    },
    Module {
        name: "test",
        prefix: "test_",
        direct: &["assert"],
    },
    Module {
        name: "json",
        prefix: "json_",
        direct: &[],
    },
    Module {
        name: "bytes",
        prefix: "bytes_",
        direct: &[],
    },
    Module {
        name: "time",
        prefix: "time_",
        direct: &["sleep"],
    },
    Module {
        name: "math",
        prefix: "math_",
        direct: &[],
    },
    Module {
        name: "crypto",
        prefix: "crypto_",
        direct: &[],
    },
    Module {
        name: "jwt",
        prefix: "jwt_",
        direct: &[],
    },
    Module {
        name: "fs",
        prefix: "fs_",
        direct: &[],
    },
    Module {
        name: "http",
        prefix: "http_",
        direct: &[],
    },
    Module {
        name: "tcp",
        prefix: "tcp_",
        direct: &[],
    },
    Module {
        name: "tls",
        prefix: "tls_",
        direct: &[],
    },
];
fn module(name: &str) -> Option<&'static Module> {
    MODULES.iter().find(|module| module.name == name)
}
pub(super) fn contains(name: &str) -> bool {
    module(name).is_some()
}
pub(super) fn path(name: &str) -> PathBuf {
    PathBuf::from("<builtin>").join(name)
}
pub(super) fn is_path(path: &std::path::Path) -> bool {
    path.starts_with("<builtin>")
}
impl Module {
    fn export<'a>(&self, operation: &'a str) -> Option<&'a str> {
        if self.direct.contains(&operation) {
            Some(operation)
        } else {
            operation
                .strip_prefix(self.prefix)
                .filter(|name| !name.is_empty())
        }
    }
}
pub(super) fn operation(source: &str, export: &str, hosts: &Hosts) -> Result<String> {
    let module = module(source).ok_or_else(|| format!("unknown builtin module '{source}'"))?;
    let operation = if module.direct.contains(&export) {
        export.to_owned()
    } else {
        format!("{}{export}", module.prefix)
    };
    hosts
        .operation(&operation)
        .map_err(|_| format!("builtin module '{source}' does not export '{export}'"))?;
    Ok(operation)
}
pub(super) fn exports<'a>(
    hosts: &Hosts,
    declarations: &deka_syntax::ModuleExports<'a>,
    arena: &'a bumpalo::Bump,
) -> HashMap<PathBuf, deka_syntax::ModuleExports<'a>> {
    let operations = hosts.declarations_names_and_arities();
    MODULES
        .iter()
        .map(|module| {
            // Preserve nominal identities and receiver metadata from the single host
            // declaration pass. Only public value exports vary by module.
            let mut exports = declarations.clone();
            exports.values.clear();
            for operation in operations.keys() {
                if let Some(name) = module.export(operation)
                    && let Some(ty) = declarations.values.get(operation.as_str())
                {
                    exports.values.insert(arena.alloc_str(name), ty.clone());
                }
            }
            (path(module.name), exports)
        })
        .collect()
}
