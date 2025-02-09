use std::env;
use std::path::PathBuf;

fn main() {
    // Get the OUT_DIR environment variable at runtime
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Construct the path to the descriptor set file
    let descriptor_path = out_dir.join("endpoint_descriptor.bin");

    // Configure and compile the proto files
    tonic_build::configure()
        .protoc_arg("--experimental_allow_proto3_optional")
        .file_descriptor_set_path(descriptor_path)
        .compile_protos(&["endpoint_service.proto"], &["proto"])
        .unwrap_or_else(|e| panic!("Failed to compile proto files: {}", e));
}
