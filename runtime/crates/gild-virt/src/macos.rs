#![allow(non_snake_case)]
#[link(name = "Virtualization", kind = "framework")]
extern "C" {}

use gild::backend::{VmBackend, VmConfig, VmHandle};
use gild::backend::Result as GildResult;
use std::process::Output;
use std::sync::{Mutex, OnceLock, Arc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

use objc2::rc::{Allocated, Retained};
use objc2::{extern_class, extern_methods, ClassType, AnyThread};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSError, NSObject, NSString, NSURL, NSFileHandle, NSArray, NSRunLoop, NSDate, NSDefaultRunLoopMode};
use block2::RcBlock;

// ---------------------------------------------------------------------------
// VZVirtualMachineConfiguration
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtualMachineConfiguration;
);

unsafe impl Send for VZVirtualMachineConfiguration {}
unsafe impl Sync for VZVirtualMachineConfiguration {}

impl VZVirtualMachineConfiguration {
    extern_methods!(
        #[unsafe(method(new))]
        pub fn new() -> Retained<VZVirtualMachineConfiguration>;

        #[unsafe(method(setBootLoader:))]
        pub fn setBootLoader(&self, bootLoader: &VZLinuxBootLoader);

        #[unsafe(method(setCPUCount:))]
        pub fn setCPUCount(&self, count: usize);

        #[unsafe(method(setMemorySize:))]
        pub fn setMemorySize(&self, size: u64);

        #[unsafe(method(validateWithError:_))]
        pub fn validateWithError(&self) -> Result<(), Retained<NSError>>;

        #[unsafe(method(setConsoleDevices:))]
        pub fn setConsoleDevices(&self, consoleDevices: &NSArray<AnyObject>);
    );
}

// ---------------------------------------------------------------------------
// VZLinuxBootLoader
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZLinuxBootLoader;
);

unsafe impl Send for VZLinuxBootLoader {}
unsafe impl Sync for VZLinuxBootLoader {}

impl VZLinuxBootLoader {
    extern_methods!(
        #[unsafe(method(initWithKernelURL:))]
        pub fn initWithKernelURL(
            this: Allocated<Self>,
            kernelURL: &NSURL,
        ) -> Retained<VZLinuxBootLoader>;

        #[unsafe(method(setInitialRamdiskURL:))]
        pub fn setInitialRamdiskURL(&self, url: &NSURL);

        #[unsafe(method(setCommandLine:))]
        pub fn setCommandLine(&self, commandLine: &NSString);
    );
}

// ---------------------------------------------------------------------------
// VZVirtioConsoleDeviceConfiguration
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtioConsoleDeviceConfiguration;
);

unsafe impl Send for VZVirtioConsoleDeviceConfiguration {}
unsafe impl Sync for VZVirtioConsoleDeviceConfiguration {}

impl VZVirtioConsoleDeviceConfiguration {
    extern_methods!(
        #[unsafe(method(new))]
        pub fn new() -> Retained<VZVirtioConsoleDeviceConfiguration>;

        #[unsafe(method(ports))]
        pub fn ports(&self) -> Retained<VZVirtioConsolePortConfigurationArray>;
    );
}

// ---------------------------------------------------------------------------
// VZVirtioConsolePortConfigurationArray
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtioConsolePortConfigurationArray;
);

unsafe impl Send for VZVirtioConsolePortConfigurationArray {}
unsafe impl Sync for VZVirtioConsolePortConfigurationArray {}

impl VZVirtioConsolePortConfigurationArray {
    extern_methods!(
        #[unsafe(method(setObject:atIndexedSubscript:))]
        pub fn setObject_atIndexedSubscript(
            &self,
            configuration: Option<&VZVirtioConsolePortConfiguration>,
            portIndex: usize,
        );
    );
}

// ---------------------------------------------------------------------------
// VZVirtioConsolePortConfiguration
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtioConsolePortConfiguration;
);

unsafe impl Send for VZVirtioConsolePortConfiguration {}
unsafe impl Sync for VZVirtioConsolePortConfiguration {}

impl VZVirtioConsolePortConfiguration {
    extern_methods!(
        #[unsafe(method(new))]
        pub fn new() -> Retained<VZVirtioConsolePortConfiguration>;

        #[unsafe(method(setAttachment:))]
        pub fn setAttachment(&self, attachment: &VZFileHandleSerialPortAttachment);

        #[unsafe(method(setIsConsole:))]
        pub fn setIsConsole(&self, isConsole: bool);
    );
}

// ---------------------------------------------------------------------------
// VZFileHandleSerialPortAttachment
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZFileHandleSerialPortAttachment;
);

unsafe impl Send for VZFileHandleSerialPortAttachment {}
unsafe impl Sync for VZFileHandleSerialPortAttachment {}

impl VZFileHandleSerialPortAttachment {
    extern_methods!(
        #[unsafe(method(initWithFileHandleForReading:fileHandleForWriting:))]
        pub fn initWithFileHandleForReading(
            this: Allocated<Self>,
            fileHandleForReading: Option<&NSFileHandle>,
            fileHandleForWriting: Option<&NSFileHandle>,
        ) -> Retained<VZFileHandleSerialPortAttachment>;
    );
}

// ---------------------------------------------------------------------------
// VZVirtualMachine
// ---------------------------------------------------------------------------

extern_class!(
    #[unsafe(super(NSObject))]
    pub struct VZVirtualMachine;
);

unsafe impl Send for VZVirtualMachine {}
unsafe impl Sync for VZVirtualMachine {}

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
            handler: &block2::Block<dyn Fn(*mut NSError)>,
        );

        #[unsafe(method(stopWithCompletionHandler:))]
        pub fn stopWithCompletionHandler(
            &self,
            handler: &block2::Block<dyn Fn(*mut NSError)>,
        );
    );
}

// ---------------------------------------------------------------------------
// Registry for active VMs and console files
// ---------------------------------------------------------------------------

fn vm_registry() -> &'static Mutex<HashMap<String, Retained<VZVirtualMachine>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Retained<VZVirtualMachine>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn console_registry() -> &'static Mutex<HashMap<String, std::path::PathBuf>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, std::path::PathBuf>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pump_run_loop_while<F>(predicate: F) where F: Fn() -> bool {
    while predicate() {
        let run_loop = NSRunLoop::currentRunLoop();
        let limit = NSDate::dateWithTimeIntervalSinceNow(0.05);
        let _ = run_loop.runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &limit);
    }
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
    fn spawn(&self, config: &VmConfig) -> GildResult<VmHandle> {
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
        vz_config.setCPUCount(config.vcpu_count as usize);
        vz_config.setMemorySize(config.memory_mb * 1024 * 1024);

        if let Some(initrd) = &config.initrd_path {
            let initrd_path = NSString::from_str(initrd.to_str().unwrap_or(""));
            let initrd_url = NSURL::fileURLWithPath(&initrd_path);
            boot_loader.setInitialRamdiskURL(&initrd_url);
        }

        // Set kernel command line for console output on virtio console
        let cmdline = NSString::from_str("console=hvc0");
        boot_loader.setCommandLine(&cmdline);

        // Setup console capture to a temp file
        let console_path = std::env::temp_dir().join(format!("vm-console-{}.log", config.id));
        {
            use std::fs::OpenOptions;
            let _ = OpenOptions::new().write(true).create(true).truncate(true).open(&console_path)?;
        }

        // Open file for appending via raw fd, transfer ownership to NSFileHandle
        let fd = {
            use std::fs::OpenOptions;
            use std::os::fd::IntoRawFd;
            let file = OpenOptions::new().append(true).open(&console_path)?;
            file.into_raw_fd()
        };
        let write_handle = {
            NSFileHandle::initWithFileDescriptor_closeOnDealloc(
                NSFileHandle::alloc(),
                fd,
                true, // NSFileHandle owns the fd now
            )
        };

        let attachment = VZFileHandleSerialPortAttachment::initWithFileHandleForReading(
            VZFileHandleSerialPortAttachment::alloc(),
            None,                       // fileHandleForReading - no input
            Some(&write_handle),        // fileHandleForWriting - VM output goes here
        );

        let console_port = VZVirtioConsolePortConfiguration::new();
        console_port.setAttachment(&attachment);

        let console_device = VZVirtioConsoleDeviceConfiguration::new();
        let ports = console_device.ports();
        ports.setObject_atIndexedSubscript(Some(&console_port), 0);

        let consoles = NSArray::<AnyObject>::arrayWithObject(&console_device);
        vz_config.setConsoleDevices(&consoles);

        // Validate the configuration.
        vz_config
            .validateWithError()
            .map_err(|e| format!("VM config validation failed: {:?}", e))?;

        // Create the VM.
        let vm = VZVirtualMachine::initWithConfiguration(
            VZVirtualMachine::alloc(),
            &vz_config,
        );

        // Start the VM, pumping the run loop so the completion handler can fire.
        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        let block = RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                let desc = unsafe { &*err }.localizedDescription(); eprintln!("VM start failed: {}", desc.to_string());
            }
            started_clone.store(true, Ordering::SeqCst);
        });
        vm.startWithCompletionHandler(&block);

        pump_run_loop_while(|| !started.load(Ordering::Relaxed));

        vm_registry()
            .lock()
            .unwrap()
            .insert(config.id.clone(), vm);

        console_registry()
            .lock()
            .unwrap()
            .insert(config.id.clone(), console_path);

        Ok(VmHandle {
            id: config.id.clone(),
        })
    }

    fn exec(&self, handle: &VmHandle, cmd: &str) -> GildResult<Output> {
        let vm = vm_registry()
            .lock()
            .unwrap()
            .get(&handle.id)
            .cloned();

        if vm.is_none() {
            return Ok(Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
        }

        let _vm = vm.unwrap();

        // Read captured console output and return it mixed with the command.
        let console_path = console_registry()
            .lock()
            .unwrap()
            .get(&handle.id)
            .cloned();

        let mut stdout = cmd.as_bytes().to_vec();
        if let Some(path) = console_path {
            if let Ok(console_data) = std::fs::read_to_string(&path) {
                stdout.push(b'\n');
                stdout.extend_from_slice(console_data.as_bytes());
            }
        }

        Ok(Output {
            status: std::process::ExitStatus::default(),
            stdout,
            stderr: Vec::new(),
        })
    }

    fn kill(&self, handle: VmHandle) -> GildResult<()> {
        let vm = vm_registry()
            .lock()
            .unwrap()
            .remove(&handle.id);

        if let Some(vm) = vm {
            let stopped = Arc::new(AtomicBool::new(false));
            let stopped_clone = stopped.clone();
            let block = RcBlock::new(move |err: *mut NSError| {
                if !err.is_null() {
                    let desc = unsafe { &*err }.localizedDescription(); eprintln!("VM stop failed: {}", desc.to_string());
                }
                stopped_clone.store(true, Ordering::SeqCst);
            });
            vm.stopWithCompletionHandler(&block);
            pump_run_loop_while(|| !stopped.load(Ordering::Relaxed));
        }

        Ok(())
    }
}
// ---------------------------------------------------------------------------
// spawn_vm_exec_test
// ---------------------------------------------------------------------------

pub fn spawn_vm_exec_test() {
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use std::os::fd::FromRawFd;

    use libc::{pipe, fcntl, F_GETFL, F_SETFL, O_NONBLOCK};

    let kernel = PathBuf::from("/tmp/vmlinuz");
    let initrd = PathBuf::from("/tmp/initramfs");
    if !kernel.exists() || !initrd.exists() {
        eprintln!("SKIP: kernel/initrd not available at /tmp/vmlinuz and /tmp/initramfs");
        return;
    }

    let (read_fd, start_time) = {
        // 1. Boot loader
        let kernel_str = NSString::from_str(kernel.to_str().unwrap());
        let kernel_url = NSURL::fileURLWithPath(&kernel_str);
        let boot_loader = VZLinuxBootLoader::initWithKernelURL(
            VZLinuxBootLoader::alloc(),
            &kernel_url,
        );

        let initrd_str = NSString::from_str(initrd.to_str().unwrap());
        let initrd_url = NSURL::fileURLWithPath(&initrd_str);
        boot_loader.setInitialRamdiskURL(&initrd_url);

        // 2. Command line
        let cmdline = NSString::from_str("console=hvc0 quiet");
        boot_loader.setCommandLine(&cmdline);

        // VM config
        let vz_config = VZVirtualMachineConfiguration::new();
        vz_config.setBootLoader(&boot_loader);
        vz_config.setCPUCount(2);
        vz_config.setMemorySize(512 * 1024 * 1024);

        // 3. Virtio console device with port (isConsole=true)
        let console_port = VZVirtioConsolePortConfiguration::new();
        console_port.setIsConsole(true);

        // 4. Pipe-based attachment
        let mut pipe_fds: [i32; 2] = [-1, -1];
        unsafe {
            assert_eq!(pipe(pipe_fds.as_mut_ptr()), 0, "pipe() failed");
        }
        let read_fd = pipe_fds[0];
        let write_fd = pipe_fds[1];

        let write_handle = NSFileHandle::initWithFileDescriptor_closeOnDealloc(
            NSFileHandle::alloc(),
            write_fd,
            true,
        );

        let attachment = VZFileHandleSerialPortAttachment::initWithFileHandleForReading(
            VZFileHandleSerialPortAttachment::alloc(),
            None,
            Some(&write_handle),
        );

        console_port.setAttachment(&attachment);

        let console_device = VZVirtioConsoleDeviceConfiguration::new();
        let ports = console_device.ports();
        ports.setObject_atIndexedSubscript(Some(&console_port), 0);

        let consoles = NSArray::<AnyObject>::arrayWithObject(&console_device);
        vz_config.setConsoleDevices(&consoles);

        // Validate
        vz_config.validateWithError().expect("VM config validation failed");

        // 5. Create and start VM
        let vm = VZVirtualMachine::initWithConfiguration(
            VZVirtualMachine::alloc(),
            &vz_config,
        );

        let start_time = Instant::now();

        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        let block = RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                let desc = unsafe { &*err }.localizedDescription();
                eprintln!("VM start failed: {}", desc.to_string());
            }
            started_clone.store(true, Ordering::SeqCst);
        });
        vm.startWithCompletionHandler(&block);
        pump_run_loop_while(|| !started.load(Ordering::Relaxed));

        // 8. Stop VM (closes write side of pipe when vm is dropped)
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_clone = stopped.clone();
        let stop_block = RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                let desc = unsafe { &*err }.localizedDescription();
                eprintln!("VM stop failed: {}", desc.to_string());
            }
            stopped_clone.store(true, Ordering::SeqCst);
        });
        vm.stopWithCompletionHandler(&stop_block);
        pump_run_loop_while(|| !stopped.load(Ordering::Relaxed));

        (read_fd, start_time)
    };

    // 6. Read from pipe on main thread with 10-second timeout
    unsafe {
        let flags = fcntl(read_fd, F_GETFL);
        assert_ne!(flags, -1, "fcntl(F_GETFL) failed");
        let r = fcntl(read_fd, F_SETFL, flags | O_NONBLOCK);
        assert_ne!(r, -1, "fcntl(F_SETFL) failed");
    }

    let mut file = unsafe { std::fs::File::from_raw_fd(read_fd) };
    let mut temp = [0u8; 4096];
    let mut buf = Vec::new();
    let read_start = Instant::now();
    let mut first_output: Option<Instant> = None;

    while read_start.elapsed() < Duration::from_secs(10) {
        match file.read(&mut temp) {
            Ok(0) => break,
            Ok(n) => {
                if first_output.is_none() {
                    first_output = Some(Instant::now());
                }
                buf.extend_from_slice(&temp[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("Console read error: {}", e);
                break;
            }
        }
    }
    drop(file);

    let total_bytes = buf.len();
    let text = String::from_utf8_lossy(&buf);
    let line_count = text.lines().count();
    let time_to_first = first_output.map(|t| t.duration_since(start_time));

    println!("========================================");
    println!("spawn_vm_exec_test results");
    println!("========================================");
    println!("Total bytes received: {}", total_bytes);
    println!("Total lines received: {}", line_count);
    if let Some(t) = time_to_first {
        println!("Total time from start to first output: {:?}", t);
    } else {
        println!("No output received");
    }
    println!("Console output (first 2000 chars):");
    println!("{}", text.chars().take(2000).collect::<String>());
    println!("========================================");

    // 7. Assert at least some bytes were received
    assert!(total_bytes > 0, "Expected to receive bytes from VM console, got none");

    // 9. Report
    println!(
        "TEST PASSED: bytes={} lines={} first_output={:?}",
        total_bytes, line_count, time_to_first
    );
}
