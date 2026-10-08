fn main() {
    // Android 15 loads apps on 16 KB pages, and Play requires it from SDK 35:
    // the shipped library's segments must align to 16 KB. NDK r27's linker
    // still defaults to 4 KB, so say so here; a build-script link argument
    // survives the RUSTFLAGS the Tauri CLI sets for Android targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
    tauri_build::build()
}
