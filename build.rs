use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=assets/clipwheel.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let rc = out.join("clipwheel.rc");
    let icon = root
        .join("assets/clipwheel.ico")
        .to_string_lossy()
        .replace('\\', "/");
    fs::write(&rc, format!("1 ICON \"{icon}\"\n")).unwrap();
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let resource = out.join(if msvc { "clipwheel.res" } else { "clipwheel.o" });
    let status = if msvc {
        Command::new("rc.exe")
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(&rc)
            .status()
    } else {
        Command::new(env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into()))
            .args(["-i"])
            .arg(&rc)
            .args(["-o"])
            .arg(&resource)
            .args(["-O", "coff"])
            .status()
    }
    .expect(
        "Windows resource compiler unavailable; use a Windows SDK developer shell or set WINDRES",
    );
    assert!(status.success(), "Icon resource compilation failed");
    println!("cargo:rustc-link-arg={}", resource.display());
}
