// Aurora Browser — lightweight Rust browser built with wry + tao
// License: MIT
#![windows_subsystem = "windows"]

mod app;
mod config;
mod events;
mod icon;
mod ipc;
mod portal;
mod settings;
mod toolbar;
mod webviews;

fn main() {
    app::run();
}
