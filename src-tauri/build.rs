fn main() {
    let platform=match (std::env::var("CARGO_CFG_TARGET_OS").unwrap().as_str(),std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str()) {
        ("windows","x86_64")=>"win32-x64",("macos","x86_64")=>"darwin-x64",("macos","aarch64")=>"darwin-arm64",("linux","x86_64")=>"linux-x64",
        _=>panic!("Prepare a supported sing-box desktop target before building"),
    };
    let source=std::path::PathBuf::from("../.tools/singbox").join(platform);
    println!("cargo:rerun-if-changed={}",source.join("core.gz").display());
    println!("cargo:rerun-if-changed={}",source.join("core.sha256").display());
    for file in ["core.gz","core.sha256"] {
        std::fs::copy(source.join(file),std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join(file))
            .expect("Run node scripts/prepare-singbox.mjs before cargo/tauri builds");
    }
    tauri_build::build()
}
