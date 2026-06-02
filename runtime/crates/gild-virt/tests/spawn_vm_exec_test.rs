#![cfg(all(feature = "macos", target_os = "macos"))]
use gild_virt::spawn_vm_exec_test;

fn main() {
    spawn_vm_exec_test();
}
