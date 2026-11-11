fn main() {
    slint_build::compile_with_config(
        "ui/main.slint",
        // One style everywhere, so the menus look the same on every machine.
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .expect("Slint UI failed to compile");

    // The icon inside the .exe, which Explorer, the taskbar and `main.rs` use.
    // Only on a Windows host, which has the resource compiler: CI builds the .exe there.
    println!("cargo:rerun-if-changed=../../../desktop/favicon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../../desktop/favicon.ico");
        // The version tab in Properties, and the name Task Manager and the "Windows
        // protected your PC" prompt show. The version numbers come from Cargo.toml.
        res.set("ProductName", "LRC Open Player");
        res.set("FileDescription", "LRC Open Player");
        res.set("OriginalFilename", "LRCOpenPlayer.exe");
        res.set("InternalName", "LRCOpenPlayer");
        res.set("LegalCopyright", "Free software under the GNU GPL v3.0");
        res.compile().expect("could not add the icon to the .exe");
    }
}
