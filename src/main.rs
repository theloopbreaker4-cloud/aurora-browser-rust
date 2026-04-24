// Aurora Browser — lightweight Rust browser built with wry + tao
// License: MIT
#![windows_subsystem = "windows"]

mod about;
mod app;
mod bookmarks_page;
mod config;
mod downloads_page;
mod error;
mod events;
mod history;
mod icon;
mod ipc;
mod portal;
#[cfg(feature = "servo-engine")]
mod servo_view;
mod settings;
mod test_page;
mod toolbar;
mod webviews;

fn main() {
    app::run();
}
