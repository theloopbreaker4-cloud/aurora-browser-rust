// Aurora Browser — lightweight Rust browser built with wry + tao
// License: MIT
#![windows_subsystem = "windows"]

mod app;
mod config;
mod engine;
mod events;
mod ipc;
mod pages;
mod platform;
mod toolbar;
mod webviews;

// Legacy re-export shim so app.rs can still use crate::servo_view::
#[cfg(feature = "servo-engine")]
mod servo_view;

fn main() {
    app::run();
}
