use std::path::PathBuf;
use std::process::Output;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Configuration for spawning a VM.
#[derive(Debug, Clone)]
pub struct VmConfig {
    pub id: String,
    pub kernel_path: PathBuf,
    pub initrd_path: Option<PathBuf>,
    pub rootfs_path: PathBuf,
    pub memory_mb: u64,
    pub vcpu_count: u32,
}

/// Opaque handle to a running VM.
#[derive(Debug, Clone)]
pub struct VmHandle {
    pub id: String,
}

/// Abstraction over hypervisor backends (Firecracker on Linux,
/// Virtualization.framework on macOS).
pub trait VmBackend: Send + Sync {
    /// Spawn a new VM from the given configuration.
    fn spawn(&self, config: &VmConfig) -> Result<VmHandle>;

    /// Execute a command inside the VM.
    fn exec(&self, handle: &VmHandle, cmd: &str) -> Result<Output>;

    /// Kill and tear down the VM.
    fn kill(&self, handle: VmHandle) -> Result<()>;
}
