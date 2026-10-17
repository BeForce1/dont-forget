#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod common;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(target_os = "linux")]
    linux::main();
}
