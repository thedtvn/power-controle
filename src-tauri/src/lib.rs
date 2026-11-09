mod battery_info;
mod bios_pwd;
mod cctk_install;
mod config;
mod dell;
mod elevation;
mod instance;
mod ipc;
mod monitor;
mod power;
mod service;
mod settings_client;
mod shell;
mod startup;
mod ui;

pub fn run() {
    if let Err(error) = shell::run() {
        // Release builds have no console: keep a diagnostic in the existing
        // per-user app directory without including configuration or secrets.
        if let Some(base) = std::env::var_os("APPDATA") {
            let dir = std::path::PathBuf::from(base).join("com.duong.power-controle");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("last-error.log"), error.to_string());
        }
        eprintln!("Power Control could not start: {error}");
        std::process::exit(1);
    }
}
