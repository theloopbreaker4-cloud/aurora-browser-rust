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
    FindPrev(String),
    Print,
    ViewSource,
    ToggleFullscreen,
    MinimizeWindow,
    MaximizeWindow,
    CloseWindow,
    DragWindow,
    SwitchEngine(String),
    /// Wakes the event loop when Servo needs to paint a new frame.
    #[cfg(feature = "servo-engine")]
    ServoWake,
}
