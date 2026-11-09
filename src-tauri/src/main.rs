// Native release tray app has no console; debug builds retain diagnostics.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    power_controle_lib::run()
}
