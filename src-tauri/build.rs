fn main() {
    #[cfg(target_os = "windows")]
    {
        if std::env::var("PROFILE").unwrap_or_default() == "release" {
            let windows = tauri_build::WindowsAttributes::new()
                .app_manifest(include_str!("windows/app.manifest"));
            let attrs = tauri_build::Attributes::new().windows_attributes(windows);
            tauri_build::try_build(attrs).expect("failed to run tauri-build");
        } else {
            tauri_build::build();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        tauri_build::build();
    }
}
