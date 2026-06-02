use gild::backend::{VmBackend, VmConfig, VmHandle, Result};
use std::process::Output;

use objc2::rc::{Allocated, Retained};
use objc2::{extern_class, extern_methods, ClassType};
use objc2_foundation::{NSError, NSObject, NSString, NSURL};
use block2::Block;

// ---------------------------------------------------------------------------
// VZVirtualMachineConfiguration
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtualMachineConfiguration;
);

impl VZVirtualMachineConfiguration {
    extern_methods!(
        #[unsafe(method(new))]
        pub fn new() -> Retained<VZVirtualMachineConfiguration>;

        #[unsafe(method(setBootLoader:))]
        pub fn setBootLoader(&self, bootLoader: &VZLinuxBootLoader);

        #[unsafe(method(setCPUsCount:))]
        pub fn setCPUsCount(&self, count: usize);

        #[unsafe(method(setMemorySize:))]
        pub fn setMemorySize(&self, size: u64);

        #[unsafe(method(validateWithError:_))]
        pub fn validateWithError(&self) -> Result<(), Retained<NSError>>;
    );
}

// ---------------------------------------------------------------------------
// VZLinuxBootLoader
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZLinuxBootLoader;
);

impl VZLinuxBootLoader {
    extern_methods!(
        #[unsafe(method(initWithKernelURL:))]
        pub fn initWithKernelURL(
            this: Allocated<Self>,
            kernelURL: &NSURL,
        ) -> Retained<VZLinuxBootLoader>;

        #[unsafe(method(setInitialRamdiskURL:))]
        pub fn setInitialRamdiskURL(&self, url: &NSURL);
    );
}

// ---------------------------------------------------------------------------
// VZVirtioConsoleDeviceConfiguration
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtioConsoleDeviceConfiguration;
);

impl VZVirtioConsoleDeviceConfiguration {
    extern_methods!(
        #[unsafe(method(new))]
        pub fn new() -> Retained<VZVirtioConsoleDeviceConfiguration>;
    );
}

// ---------------------------------------------------------------------------
// VZVirtualMachine
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtualMachine;
);

impl VZVirtualMachine {
    extern_methods!(
        #[unsafe(method(initWithConfiguration:))]
        pub fn initWithConfiguration(
            this: Allocated<Self>,
            configuration: &VZVirtualMachineConfiguration,
        ) -> Retained<VZVirtualMachine>;

        #[unsafe(method(startWithCompletionHandler:))]
        pub fn startWithCompletionHandler(
            &self,
            handler: &Block<dyn Fn(*mut NSError)>,
        );
    );
}

// ---------------------------------------------------------------------------
// VirtualizationFrameworkBackend
// ---------------------------------------------------------------------------

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
    fn spawn(&self, config: &VmConfig) -> Result<VmHandle> {
        let vz_config = VZVirtualMachineConfiguration::new();

        let kernel_path = NSString::from_str(
            config.kernel_path.to_str().unwrap_or(""),
        );
        let kernel_url = NSURL::fileURLWithPath(&kernel_path);

        let boot_loader = VZLinuxBootLoader::initWithKernelURL(
            VZLinuxBootLoader::alloc(),
            &kernel_url,
        );
        vz_config.setBootLoader(&boot_loader);
        vz_config.setCPUsCount(config.vcpu_count as usize);
        vz_config.setMemorySize(config.memory_mb * 1024 * 1024);

        if let Some(initrd) = &config.initrd_path {
            let initrd_path = NSString::from_str(initrd.to_str().unwrap_or(""));
            let initrd_url = NSURL::fileURLWithPath(&initrd_path);
            boot_loader.setInitialRamdiskURL(&initrd_url);
        }

        // Validate the configuration.
        vz_config
            .validateWithError()
            .map_err(|e| format!("VM config validation failed: {:?}", e))?;

        // Create the VM object (do not start it in this stub).
        let _vm = VZVirtualMachine::initWithConfiguration(
            VZVirtualMachine::alloc(),
            &vz_config,
        );

        Ok(VmHandle {
            id: config.id.clone(),
        })
    }

    fn exec(&self, _handle: &VmHandle, _cmd: &str) -> Result<Output> {
        Ok(Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn kill(&self, _handle: VmHandle) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn spawn_vm_noop_test() {
        let backend = VirtualizationFrameworkBackend::new();
        let tmp = std::env::temp_dir().join("test-kernel");
        std::fs::write(&tmp, b"").unwrap();
        let config = VmConfig {
            id: "test-vm".to_string(),
            kernel_path: tmp,
            initrd_path: None,
            rootfs_path: PathBuf::from("/dev/null"),
            memory_mb: 512,
            vcpu_count: 2,
        };
        let result = backend.spawn(&config);
        assert!(result.is_ok(), "VM config should be valid: {:?}", result.err());
    }
}
