use std::borrow::Cow;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deno_core::Extension;
use deno_permissions::{
    AllowRunDescriptor, AllowRunDescriptorParseResult, DenyRunDescriptor, EnvDescriptor,
    FfiDescriptor, ImportDescriptor, NetDescriptor, PathDescriptor, PathQueryDescriptor,
    PathResolveError, PermissionDescriptorParser, Permissions, PermissionsContainer,
    PermissionsOptions, ReadDescriptor, RunDescriptorParseError, RunQueryDescriptor,
    SpecialFilePathQueryDescriptor, SysDescriptor, WriteDescriptor,
};

#[derive(Debug, Clone)]
struct DekaPermissionDescriptorParser {
    cwd: PathBuf,
}

impl DekaPermissionDescriptorParser {
    fn new() -> Result<Self, PathResolveError> {
        let cwd = std::env::current_dir().map_err(PathResolveError::CwdResolve)?;
        Ok(Self { cwd })
    }

    fn resolve_path_descriptor(&self, text: &str) -> Result<PathDescriptor, PathResolveError> {
        if text.is_empty() {
            return Err(PathResolveError::EmptyPath);
        }
        Ok(PathDescriptor::new_known_cwd(
            Cow::Owned(PathBuf::from(text)),
            &self.cwd,
        ))
    }
}

impl PermissionDescriptorParser for DekaPermissionDescriptorParser {
    fn parse_read_descriptor(&self, text: &str) -> Result<ReadDescriptor, PathResolveError> {
        Ok(ReadDescriptor(self.resolve_path_descriptor(text)?))
    }

    fn parse_write_descriptor(&self, text: &str) -> Result<WriteDescriptor, PathResolveError> {
        Ok(WriteDescriptor(self.resolve_path_descriptor(text)?))
    }

    fn parse_net_descriptor(
        &self,
        text: &str,
    ) -> Result<NetDescriptor, deno_permissions::NetDescriptorParseError> {
        NetDescriptor::parse_for_list(text)
    }

    fn parse_import_descriptor(
        &self,
        text: &str,
    ) -> Result<ImportDescriptor, deno_permissions::NetDescriptorParseError> {
        ImportDescriptor::parse_for_list(text)
    }

    fn parse_env_descriptor(
        &self,
        text: &str,
    ) -> Result<EnvDescriptor, deno_permissions::EnvDescriptorParseError> {
        Ok(EnvDescriptor::new(Cow::Borrowed(text)))
    }

    fn parse_sys_descriptor(
        &self,
        text: &str,
    ) -> Result<SysDescriptor, deno_permissions::SysDescriptorParseError> {
        SysDescriptor::parse(text.to_string())
    }

    fn parse_allow_run_descriptor(
        &self,
        text: &str,
    ) -> Result<AllowRunDescriptorParseResult, RunDescriptorParseError> {
        Ok(AllowRunDescriptorParseResult::Descriptor(
            AllowRunDescriptor(self.resolve_path_descriptor(text)?),
        ))
    }

    fn parse_deny_run_descriptor(&self, text: &str) -> Result<DenyRunDescriptor, PathResolveError> {
        if text.contains(std::path::MAIN_SEPARATOR) || Path::new(text).is_absolute() {
            Ok(DenyRunDescriptor::Path(self.resolve_path_descriptor(text)?))
        } else {
            Ok(DenyRunDescriptor::Name(text.to_string()))
        }
    }

    fn parse_ffi_descriptor(&self, text: &str) -> Result<FfiDescriptor, PathResolveError> {
        Ok(FfiDescriptor(self.resolve_path_descriptor(text)?))
    }

    fn parse_path_query<'a>(
        &self,
        path: Cow<'a, Path>,
    ) -> Result<PathQueryDescriptor<'a>, PathResolveError> {
        if path.is_absolute() {
            return Ok(PathQueryDescriptor::new_known_absolute(path));
        }

        let requested = path.to_string_lossy().to_string();
        let resolved = self.cwd.join(path.as_ref());
        Ok(PathQueryDescriptor::new_known_absolute(Cow::Owned(resolved)).with_requested(requested))
    }

    fn parse_special_file_descriptor<'a>(
        &self,
        path: PathQueryDescriptor<'a>,
    ) -> Result<SpecialFilePathQueryDescriptor<'a>, PathResolveError> {
        SpecialFilePathQueryDescriptor::parse(&sys_traits::impls::RealSys, path)
    }

    fn parse_net_query(
        &self,
        text: &str,
    ) -> Result<NetDescriptor, deno_permissions::NetDescriptorParseError> {
        NetDescriptor::parse_for_query(text)
    }

    fn parse_run_query<'a>(
        &self,
        requested: &'a str,
    ) -> Result<RunQueryDescriptor<'a>, RunDescriptorParseError> {
        if AllowRunDescriptor::is_path(requested) {
            let path = Path::new(requested);
            if path.is_absolute() {
                return Ok(RunQueryDescriptor::Path(
                    PathQueryDescriptor::new_known_absolute(Cow::Owned(path.to_path_buf())),
                ));
            }
            let resolved = self.cwd.join(path);
            return Ok(RunQueryDescriptor::Path(
                PathQueryDescriptor::new_known_absolute(Cow::Owned(resolved))
                    .with_requested(requested.to_string()),
            ));
        }
        Ok(RunQueryDescriptor::Name(requested.to_string()))
    }
}

pub fn permissions_extension() -> Extension {
    Extension {
        name: "deka_permissions",
        op_state_fn: Some(Box::new(|state| {
            let parser = DekaPermissionDescriptorParser::new().unwrap_or_else(|_| {
                DekaPermissionDescriptorParser {
                    cwd: PathBuf::from("/"),
                }
            });
            let parser = Arc::new(parser);
            let container = permissions_container_from_env(parser);
            state.put(container);
        })),
        ..Default::default()
    }
}

fn permissions_container_from_env(
    parser: Arc<DekaPermissionDescriptorParser>,
) -> PermissionsContainer {
    let prompt = security_prompt_enabled();
    let opts = permissions_options_from_env_json(std::env::var("DEKA_SECURITY_POLICY").ok().as_deref());
    let perms = Permissions::from_options(parser.as_ref(), &opts).unwrap_or_else(|_| {
        if prompt {
            Permissions::none_with_prompt()
        } else {
            Permissions::none_without_prompt()
        }
    });
    PermissionsContainer::new(parser, perms)
}

fn security_prompt_enabled() -> bool {
    if std::env::var("DEKA_SECURITY_NO_PROMPT")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Map `DEKA_SECURITY_POLICY` JSON onto Deno `PermissionsOptions`.
/// `true` / empty allow list ⇒ grant the whole category. `false` / missing ⇒ deny.
/// FFI is never granted.
fn permissions_options_from_env_json(raw: Option<&str>) -> PermissionsOptions {
    let mut opts = PermissionsOptions {
        prompt: false,
        ..PermissionsOptions::default()
    };
    let Some(raw) = raw else {
        return opts;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return opts;
    };
    let security = value.get("security").unwrap_or(&value);
    opts.allow_read = rule_to_option(security.get("allow").and_then(|s| s.get("read")));
    opts.deny_read = rule_to_option(security.get("deny").and_then(|s| s.get("read")));
    opts.allow_write = rule_to_option(security.get("allow").and_then(|s| s.get("write")));
    opts.deny_write = rule_to_option(security.get("deny").and_then(|s| s.get("write")));
    opts.allow_net = rule_to_option(security.get("allow").and_then(|s| s.get("net")));
    opts.deny_net = rule_to_option(security.get("deny").and_then(|s| s.get("net")));
    opts.allow_env = rule_to_option(security.get("allow").and_then(|s| s.get("env")));
    opts.deny_env = rule_to_option(security.get("deny").and_then(|s| s.get("env")));
    opts.allow_run = rule_to_option(security.get("allow").and_then(|s| s.get("run")));
    opts.deny_run = rule_to_option(security.get("deny").and_then(|s| s.get("run")));
    opts.prompt = security
        .get("prompt")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && security_prompt_enabled();
    opts
}

fn rule_to_option(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return None;
    };
    if let Some(flag) = value.as_bool() {
        return if flag { Some(Vec::new()) } else { None };
    }
    let Some(items) = value.as_array() else {
        return None;
    };
    Some(
        items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::rule_to_option;

    #[test]
    fn rule_true_is_allow_all() {
        let value = serde_json::json!(true);
        assert_eq!(rule_to_option(Some(&value)), Some(Vec::new()));
    }

    #[test]
    fn rule_false_is_deny() {
        let value = serde_json::json!(false);
        assert_eq!(rule_to_option(Some(&value)), None);
    }

    #[test]
    fn rule_list_is_scoped() {
        let value = serde_json::json!(["./src", "./data"]);
        assert_eq!(
            rule_to_option(Some(&value)),
            Some(vec!["./src".to_string(), "./data".to_string()])
        );
    }
}
