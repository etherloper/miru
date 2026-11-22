use std::{env, path::PathBuf};

// libmpv2-sys emits `rustc-link-lib=mpv`; point the linker at the staged import library and
// copy the runtime DLL next to the built executables so `cargo run` works.
fn main() {
    println!("cargo:rerun-if-env-changed=MPV_LIB_DIR");
    let dir = env::var_os("MPV_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
                .join("../../vendor/mpv-dev")
        });
    if !dir.exists() {
        println!("cargo:warning=libmpv not staged; run scripts/setup-mpv-windows.ps1");
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());

    let dll = dir.join("libmpv-2.dll");
    println!("cargo:rerun-if-changed={}", dll.display());
    // Executables live in target/<profile>. OUT_DIR sits somewhere below it (the depth varies
    // between Cargo versions), so walk up to the folder named after the profile.
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let profile_dir = out
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == profile.as_str()))
        .map(std::path::Path::to_path_buf);
    if let (true, Some(profile_dir)) = (dll.exists(), profile_dir) {
        for dest in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
            // `deps` may not exist yet on a clean build.
            if let Err(e) = std::fs::create_dir_all(&dest) {
                println!("cargo:warning=cannot create {}: {e}", dest.display());
                continue;
            }
            if let Err(e) = std::fs::copy(&dll, dest.join("libmpv-2.dll")) {
                println!(
                    "cargo:warning=failed to copy libmpv-2.dll to {}: {e}",
                    dest.display()
                );
            }
        }
    }
}
