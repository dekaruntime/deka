use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    let protoc = protoc_bin_vendored::protoc_bin_path().expect("failed to locate protoc");
    // SAFETY: build scripts run single-threaded per crate; setting process env for prost-build is safe here.
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }
    let proto_root = manifest_dir.join("proto");
    let db_proto = proto_root.join("bridge/v1/db.proto");
    let fs_proto = proto_root.join("bridge/v1/fs.proto");
    let net_proto = proto_root.join("bridge/v1/net.proto");
    println!("cargo:rerun-if-changed={}", db_proto.display());
    println!("cargo:rerun-if-changed={}", fs_proto.display());
    println!("cargo:rerun-if-changed={}", net_proto.display());
    let mut config = prost_build::Config::new();
    config.btree_map(["."]);
    config
        .compile_protos(&[db_proto, fs_proto, net_proto], &[proto_root])
        .expect("failed to compile protobuf schemas");
}
