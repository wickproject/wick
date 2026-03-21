fn main() {
    // Only link Cronet when the feature is enabled
    if std::env::var("CARGO_FEATURE_CRONET").is_ok() {
        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
            println!("cargo:rustc-link-search=native={}/lib/darwin_arm64", dir);
            println!("cargo:rustc-link-lib=static=cronet");

            // Compile the stub for the missing symbol
            cc::Build::new()
                .file("lib/darwin_arm64/stub.c")
                .compile("cronet_stub");

            for fw in &[
                "CoreFoundation", "CoreGraphics", "CoreText", "Foundation",
                "Security", "ApplicationServices", "AppKit", "IOKit",
                "OpenDirectory", "CFNetwork", "CoreServices", "Network",
                "SystemConfiguration", "UniformTypeIdentifiers",
                "CryptoTokenKit", "LocalAuthentication",
            ] {
                println!("cargo:rustc-link-lib=framework={}", fw);
            }
            println!("cargo:rustc-link-lib=bsm");
            println!("cargo:rustc-link-lib=pmenergy");
            println!("cargo:rustc-link-lib=pmsample");
            println!("cargo:rustc-link-lib=resolv");
        } else {
            panic!("Cronet static linking is currently only supported on macOS arm64. \
                    Build without --features cronet for other platforms.");
        }
    }
}
