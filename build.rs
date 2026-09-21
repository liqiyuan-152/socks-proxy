fn main() {
    println!("cargo:rerun-if-changed=windows/app.manifest");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
        let manifest = std::fs::read(root.join("windows/app.manifest")).unwrap();
        let resource =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("app-manifest.res");
        std::fs::write(&resource, manifest_resource(&manifest)).unwrap();
        println!(
            "cargo:rustc-link-arg-bin=socks-proxy={}",
            resource.display()
        );
    }
}

fn manifest_resource(manifest: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    resource_header(&mut output, 0, 0, 0, 0);
    resource_header(&mut output, manifest.len() as u32, 24, 1, 0x0409);
    output.extend_from_slice(manifest);
    while output.len() % 4 != 0 {
        output.push(0);
    }
    output
}

fn resource_header(
    output: &mut Vec<u8>,
    data_size: u32,
    resource_type: u16,
    name: u16,
    language: u16,
) {
    output.extend_from_slice(&data_size.to_le_bytes());
    output.extend_from_slice(&32u32.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&resource_type.to_le_bytes());
    output.extend_from_slice(&0xffffu16.to_le_bytes());
    output.extend_from_slice(&name.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0x1030u16.to_le_bytes());
    output.extend_from_slice(&language.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
}
