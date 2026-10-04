//! Generates Rust types from the vendored `.proto` using the vendored `protoc`
//! binary, so neither developers nor CI need a system protoc.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    println!("cargo:rerun-if-changed=proto/fxvps_client_v1.proto");
    let mut cfg = prost_build::Config::new();
    cfg.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .message_attribute(".", "#[serde(default)]")
        .type_attribute(
            ".fxvps.client.v1.Envelope.body",
            "#[serde(rename_all = \"snake_case\")]",
        );
    cfg.compile_protos(&["proto/fxvps_client_v1.proto"], &["proto"])?;
    Ok(())
}
