fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/fastium.res");
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg={}/assets/fastium.res", manifest_dir.replace('\\', "/"));
    }
}
