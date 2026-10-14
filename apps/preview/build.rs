//! Embeds the Open Write and Open Sheet icons and version strings into the two preview
//! executables (one resource each, so each exe gets its own icon).

use std::path::PathBuf;

fn embed(bin: &str, icon: &str, name: &str, exe: &str) {
    println!("cargo:rerun-if-changed={icon}");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join(bin);
    if std::fs::create_dir_all(&out).is_err() {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_output_directory(&out.to_string_lossy());
    res.set_icon(icon);
    res.set("FileDescription", &format!("{name} (next)"));
    res.set("ProductName", name);
    res.set("CompanyName", open365_branding::SUITE);
    res.set("InternalName", exe);
    res.set("OriginalFilename", &format!("{bin}.exe"));
    if let Err(e) = res.compile_for(bin) {
        println!("cargo:warning=could not embed the Windows resource for {bin}: {e}");
    }
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed(
        "open365-writer-next",
        "../../assets/icons/open-write.ico",
        open365_branding::WRITE_NAME,
        open365_branding::WRITE_EXE,
    );
    embed(
        "open365-sheets-next",
        "../../assets/icons/open-sheet.ico",
        open365_branding::SHEET_NAME,
        open365_branding::SHEET_EXE,
    );
}
