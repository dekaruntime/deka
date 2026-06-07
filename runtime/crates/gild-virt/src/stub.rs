use gild::backend::{VmBackend, VmConfig, VmHandle, Result};
use std::process::Output;

pub struct VirtualizationFrameworkBackend;

impl Default for VirtualizationFrameworkBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualizationFrameworkBackend {
    pub fn new() -> Self {
        Self
    }
}

impl VmBackend for VirtualizationFrameworkBackend {
    fn spawn(&self, _config: &VmConfig) -> Result<VmHandle> {
        Err("Virtualization.framework backend is only available on macOS".into())
    }

    fn exec(&self, _handle: &VmHandle, _cmd: &str) -> Result<Output> {
        Err("Virtualization.framework backend is only available on macOS".into())
    }

    fn kill(&self, _handle: VmHandle) -> Result<()> {
        Err("Virtualization.framework backend is only available on macOS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gild::backend::VmConfig;
    use std::path::PathBuf;

    #[test]
    fn stub_returns_err() {
        let backend = VirtualizationFrameworkBackend::new();
        let config = VmConfig {
            id: "test".to_string(),
            kernel_path: PathBuf::from("/dev/null"),
            initrd_path: None,
            rootfs_path: PathBuf::from("/dev/null"),
            memory_mb: 512,
            vcpu_count: 2,
        };
        assert!(backend.spawn(&config).is_err());
    }
}
