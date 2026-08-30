// No console window when launching loom.exe on Windows.
#![windows_subsystem = "windows"]

mod app;
mod assets;
mod model;
mod platform;
mod session;
mod shared;
mod terminal;
mod ui;

fn main() {
    app::run();
}
