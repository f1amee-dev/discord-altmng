// hide the console window on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    alt_mngr_lib::run()
}
