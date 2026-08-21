use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deno_core::Extension;
use deno_permissions::{
    AllowRunDescriptor, AllowRunDescriptorParseResult, DenyRunDescriptor, EnvDescriptor,
    FfiDescriptor, ImportDescriptor, NetDescriptor, PathDescriptor, PathQueryDescriptor,
    PathResolveError, PermissionDescriptorParser, PermissionsContainer, ReadDescriptor,
    RunDescriptorParseError, RunQueryDescriptor, SpecialFilePathQueryDescriptor, SysDescriptor,
    WriteDescriptor,
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
            let container = PermissionsContainer::allow_all(Arc::new(parser));
            state.put(container);
        })),
        ..Default::default()
    }
}
