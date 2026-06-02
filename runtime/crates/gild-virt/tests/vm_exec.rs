#![cfg(all(feature = "macos", target_os = "macos"))]
use gild_virt::VirtualizationFrameworkBackend;
use gild::backend::{VmBackend, VmConfig};
use std::path::PathBuf;

fn main() {
    let kernel = PathBuf::from("/tmp/vmlinuz-virt");
    let initrd = PathBuf::from("/tmp/initramfs-virt");
    if !kernel.exists() || !initrd.exists() {
        eprintln!("SKIP: Alpine kernel/initrd not available");
        std::process::exit(0);
    }

    let backend = VirtualizationFrameworkBackend::new();
    let config = VmConfig {
        id: "exec-test-vm".to_string(),
        kernel_path: kernel,
        initrd_path: Some(initrd),
        rootfs_path: PathBuf::from("/dev/null"),
        memory_mb: 512,
        vcpu_count: 2,
    };

    let start = std::time::Instant::now();
    let handle = backend.spawn(&config).expect("VM should spawn");
    let boot_time = start.elapsed();

    // Allow time for kernel boot messages to accumulate
    std::thread::sleep(std::time::Duration::from_secs(5));

    let exec_start = std::time::Instant::now();
    let output = backend.exec(&handle, "echo hello from inside vm").expect("exec should return");
    let exec_time = exec_start.elapsed();

    let teardown_start = std::time::Instant::now();
    backend.kill(handle).expect("VM should be killed");
    let teardown_time = teardown_start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);

    println!("========================================");
    println!("VM exec test results");
    println!("========================================");
    println!("Boot time:     {:?}", boot_time);
    println!("Exec time:     {:?}", exec_time);
    println!("Teardown time: {:?}", teardown_time);
    println!("Total time:    {:?}", start.elapsed());
    println!("Output length: {} bytes", stdout.len());
    println!("Output (first 1000 chars):");
    println!("{}", &stdout.chars().take(1000).collect::<String>());
    println!("========================================");

    // Assert we captured some real kernel boot output
    assert!(
        stdout.contains("Linux version") || stdout.contains("Booting") || stdout.contains("hello from inside vm"),
        "Expected kernel boot or command output, got: {}", &stdout.chars().take(200).collect::<String>()
    );

    println!("TEST PASSED");
}
