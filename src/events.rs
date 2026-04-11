// events.rs — UserEvent enum for IPC communication between WebViews and Rust
#[derive(Debug)]
pub enum UserEvent {
    Navigate(String),
    GoBack,
    GoForward,
    Reload,
    Stop,
    Restart,
    UpdateUrl(String),
    UpdateTitle(String),
    LoadStart,
    LoadEnd,
    OpenDevTools,
    SetZoom(f64),
    FindText(String),
    Print,
    ViewSource,
    ToggleFullscreen,
}
