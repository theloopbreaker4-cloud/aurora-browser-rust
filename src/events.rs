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
    /// Apply theme name (aurora-dark / aurora-light / aurora-sleep) to the toolbar live.
    ApplyTheme(String),
    /// Bookmark current page (title, url) into bookmarks.json.
    AddBookmark(String, String),
    /// Wipe history.json.
    ClearHistory,
    /// Wakes the event loop when Servo needs to paint a new frame.
    #[cfg(feature = "servo-engine")]
    ServoWake,
    /// IPC message from Servo-hosted aurora:// page (intercepted via aurora-ipc: scheme).
    #[cfg(feature = "servo-engine")]
    AuroraIpc(String),
}
