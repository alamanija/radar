//! Embeds `assets/icons/icon.ico` as the Windows exe icon. GPUI's Windows
//! backend loads the window icon from resource id 1 of the running exe.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");
    #[cfg(windows)]
    windows_icon();
}

#[cfg(windows)]
fn windows_icon() {
    use std::path::PathBuf;
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest.join("assets").join("icons").join("icon.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc = out.join("radar.rc");
    // rc string literals: backslashes must be doubled.
    let ico = ico.display().to_string().replace('\\', "\\\\");
    std::fs::write(&rc, format!("1 ICON \"{ico}\"\n")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
