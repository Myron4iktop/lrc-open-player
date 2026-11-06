fn main() {
    slint_build::compile("ui/main.slint").expect("Slint UI failed to compile");

    // The icon inside the .exe, which Explorer, the taskbar and `main.rs` use.
    // Only on a Windows host, which has the resource compiler: CI builds the .exe there.
    println!("cargo:rerun-if-changed=../../../desktop/favicon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../../desktop/favicon.ico");
        res.compile().expect("could not add the icon to the .exe");
    }
}
