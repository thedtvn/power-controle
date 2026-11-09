fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    // Only the application icon is embedded here. gpui-component enables GPUI's
    // default `windows-manifest` feature, which already embeds the executable's
    // only allowed manifest (DPI awareness + common controls). A second manifest
    // fails to link (CVT1100), so administrator rights are requested at runtime
    // instead; see src/elevation.rs.
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("icons/icon.ico");
        resource.compile().expect("could not embed Windows app resources");
    }
}
