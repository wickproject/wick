fn main() {
    if std::env::var("CARGO_FEATURE_CRONET").is_ok() {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            // macOS arm64: static link libcronet.a
            println!("cargo:rustc-link-search=native={}/lib/darwin_arm64", dir);
            println!("cargo:rustc-link-lib=static=cronet");

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

        } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            // Linux amd64: dynamic link libcronet.so
            // The .so must be next to the binary at runtime, or in LD_LIBRARY_PATH
            println!("cargo:rustc-link-search=native={}/lib/linux_amd64", dir);
            println!("cargo:rustc-link-lib=dylib=cronet");

        } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
            // Linux arm64: dynamic link libcronet.so
            println!("cargo:rustc-link-search=native={}/lib/linux_arm64", dir);
            println!("cargo:rustc-link-lib=dylib=cronet");

        } else {
            panic!(
                "Cronet linking not configured for this platform. \
                 Build without --features cronet, or add support in build.rs."
            );
        }
    }
}
