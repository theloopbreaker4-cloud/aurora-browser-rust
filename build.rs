#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/aurora.ico");
    res.compile().unwrap();

    // Auto-copy EGL DLLs for servo-engine feature
    #[cfg(feature = "servo-engine")]
    copy_egl_dlls();
}

#[cfg(windows)]
#[cfg(feature = "servo-engine")]
fn copy_egl_dlls() {
    use std::path::Path;

    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    // OUT_DIR is like: target/debug/build/aurora-xxx/out
    // We want: target/debug/
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    // Servo's own target/debug where ANGLE DLLs land after servo builds
    let servo_target = Path::new("../ServoRust/servo/target/debug");

    for dll in &["libEGL.dll", "libGLESv2.dll"] {
        let src = servo_target.join(dll);
        let dst = target_dir.join(dll);
        if src.exists() && !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
            println!("cargo:warning=Copied {} to {}", dll, target_dir.display());
        }
    }
    // Also try release dir if needed
    let src_rel = Path::new("../ServoRust/servo/target/release");
    if target_dir.ends_with("release") {
        for dll in &["libEGL.dll", "libGLESv2.dll"] {
            let src = src_rel.join(dll);
            let dst = target_dir.join(dll);
            if src.exists() && !dst.exists() {
                let _ = std::fs::copy(&src, &dst);
                println!("cargo:warning=Copied {} to {}", dll, target_dir.display());
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {}
