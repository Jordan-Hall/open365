//! Embeds the Open Write icon and version strings into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/open-write.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icons/open-write.ico");
    res.set("FileDescription", open365_branding::WRITE_NAME);
    res.set("ProductName", open365_branding::WRITE_NAME);
    res.set("CompanyName", open365_branding::SUITE);
    res.set("InternalName", open365_branding::WRITE_EXE);
    res.set("OriginalFilename", "open-write.exe");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed the Windows resource: {e}");
    }
}
