//! Compiles the virtual display shim (src/vdisplay.m).
fn main() {
    println!("cargo:rerun-if-changed=src/vdisplay.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/vdisplay.m")
            .flag("-fobjc-arc")
            .flag("-Wall")
            .compile("vdisplay");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
    }
}
