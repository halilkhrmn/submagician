fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");

    // Windows: the icon and version Explorer shows for the .exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("ProductName", "SubMagician").set("FileDescription", "SubMagician");
        if let Err(e) = res.compile() {
            // A cross-check from Linux may lack the resource compiler; the program works without.
            println!("cargo:warning=no Windows resources: {e}");
        }
    }
}
