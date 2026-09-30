fn main() {
    // The triple this build targets, which Tauri appends to sidecar names.
    println!("cargo:rustc-env=MAYA_TARGET_TRIPLE={}", std::env::var("TARGET").unwrap());
    tauri_build::build()
}
