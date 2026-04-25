// servo_view.rs — Embedded Servo engine for Aurora
// Replaces wry content WebView when engine=servo is selected.
//
// Architecture:
//   Aurora tao window → child Win32 HWND (content area only) → WindowRenderingContext (EGL/ANGLE)
//   ServoBuilder → Servo → WebViewBuilder → WebView
//   tao events → servo InputEvent (mouse, keyboard, scroll, resize)
//
// Rendering pattern (matches winit_minimal.rs official example):
//   - spin_event_loop() on every tao event
//   - paint() + present() only when notify_new_frame_ready fires (delegate sets needs_paint flag)
//
// Child window: Servo renders into a child HWND positioned below the toolbar,
//   so it never covers Aurora's wry toolbar.

#![cfg(feature = "servo-engine")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use dpi::PhysicalSize;
use raw_window_handle::{
    DisplayHandle, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};
use servo::{
    AllowOrDenyRequest, ContextMenuAction, ContextMenuItem as ServoContextMenuItem,
    Cursor as ServoCursor, DeviceIndependentPixel, DevicePoint, EmbedderControl, EventLoopWaker,
    InputEvent, KeyboardEvent as ServoKeyboardEvent, LoadStatus, MouseButton as ServoMouseButton,
    MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NavigationRequest, Notification,
    PermissionFeature, PermissionRequest, RenderingContext, SelectElementOptionOrOptgroup,
    ServoBuilder, ServoUrl, SimpleDialog as ServoSimpleDialog, WebView, WebViewBuilder,
    WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use servo::protocol_handler::ProtocolHandlerRegistration;
use euclid::Scale;
use tao::event_loop::EventLoopProxy;
use tao::window::Window;

use crate::events::UserEvent;

// ── Win32 child window ────────────────────────────────────────────────────

#[cfg(windows)]
mod child_window {
    use std::ptr;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, WS_CHILD, WS_CLIPCHILDREN,
        WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    // Use the built-in "STATIC" window class — no registration needed, supports child windows.
    // We override the background to black (null brush = transparent/inherit).
    const CLASS_NAME: &[u16] = &[
        b'S' as u16,
        b'T' as u16,
        b'A' as u16,
        b'T' as u16,
        b'I' as u16,
        b'C' as u16,
        0u16,
    ];

    /// Create a child HWND inside `parent_hwnd` at position (x, y) with given size.
    /// Returns the child HWND as isize (0 on failure).
    pub fn create(parent_hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> isize {
        let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
        let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
        unsafe {
            CreateWindowExW(
                0,
                CLASS_NAME.as_ptr(),
                ptr::null(),
                style,
                x,
                y,
                width,
                height,
                parent_hwnd as HWND,
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            ) as isize
        }
    }

    /// Resize/reposition child window.
    pub fn set_bounds(hwnd: isize, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            SetWindowPos(
                hwnd as HWND,
                ptr::null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

// ── Handles wrapper ───────────────────────────────────────────────────────

/// Wraps a raw HWND so we can implement HasWindowHandle / HasDisplayHandle
/// without needing a full tao Window.
struct RawHwnd(isize);

impl HasWindowHandle for RawHwnd {
    fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
        let mut h = Win32WindowHandle::new(
            std::num::NonZeroIsize::new(self.0).expect("child HWND is zero"),
        );
        h.hinstance = None; // surfman resolves it internally
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(h)) })
    }
}

impl HasDisplayHandle for RawHwnd {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe {
            DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new()))
        })
    }
}

// ── Delegate ──────────────────────────────────────────────────────────────

struct ServoState {
    needs_paint: Cell<bool>,
    proxy: EventLoopProxy<UserEvent>,
    /// Parent (Aurora) HWND used as the owner for native modal dialogs (file picker etc.).
    parent_hwnd: isize,
    /// `http://127.0.0.1:<port>` of the loopback server, if any. URLs that
    /// start with this prefix are rewritten back to `aurora://<path>` for
    /// display in the toolbar address bar — the user sees the friendly URL
    /// even though Servo is loading the loopback alias.
    aurora_origin: Option<String>,
}

struct AuroraDelegate {
    state: Rc<ServoState>,
    /// Servo child HWND (cached for client_to_screen conversion).
    child_hwnd: isize,
    /// Toolbar physical-pixel offset so we map content coords to window coords.
    toolbar_phys: u32,
}

impl AuroraDelegate {
    /// Convert Servo content-area coords (relative to the Servo render surface)
    /// to absolute screen coords for popup positioning.
    #[cfg(windows)]
    fn client_to_screen(&self, x: i32, y: i32) -> (i32, i32) {
        use windows_sys::Win32::Foundation::{HWND, POINT};
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        let mut pt = POINT { x, y };
        unsafe {
            ClientToScreen(self.child_hwnd as HWND, &mut pt);
        }
        (pt.x, pt.y)
    }
    #[cfg(not(windows))]
    fn client_to_screen(&self, x: i32, y: i32) -> (i32, i32) {
        (x, y)
    }

    #[cfg(windows)]
    fn handle_simple_dialog(&self, dialog: ServoSimpleDialog) {
        use crate::dialogs::{simple_dialog, SimpleDialogKind, SimpleDialogResult};
        let kind = match &dialog {
            ServoSimpleDialog::Alert(_) => SimpleDialogKind::Alert,
            ServoSimpleDialog::Confirm(_) => SimpleDialogKind::Confirm,
            ServoSimpleDialog::Prompt(_) => SimpleDialogKind::Prompt,
        };
        let message = dialog.message().to_string();
        let result = simple_dialog(self.state.parent_hwnd, kind, &message, None);
        match (kind, result) {
            (SimpleDialogKind::Alert, _) => dialog.confirm(),
            (SimpleDialogKind::Confirm, SimpleDialogResult::Confirmed(true)) => dialog.confirm(),
            (SimpleDialogKind::Confirm, _) => dialog.dismiss(),
            (SimpleDialogKind::Prompt, SimpleDialogResult::Prompted(Some(_))) => dialog.confirm(),
            (SimpleDialogKind::Prompt, _) => dialog.dismiss(),
        }
    }
}

impl WebViewDelegate for AuroraDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        // Throttled logging — first 5 frames only, otherwise this fills the log fast.
        let n = FRAME_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n < 5 {
            slog(&format!("notify_new_frame_ready (#{n}) — needs_paint = true"));
        }
        self.state.needs_paint.set(true);
        let _ = self.state.proxy.send_event(UserEvent::ServoWake);
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        if let Some(t) = title {
            let _ = self.state.proxy.send_event(UserEvent::UpdateTitle(t));
        }
    }

    fn request_protocol_handler(
        &self,
        _webview: WebView,
        _registration: ProtocolHandlerRegistration,
        request: AllowOrDenyRequest,
    ) {
        // Trust pages to register protocol handlers in this trial environment.
        // A future iteration should pop a real consent dialog (see roadmap).
        request.allow();
    }

    fn request_permission(&self, _webview: WebView, request: PermissionRequest) {
        // Trust-by-default for the test environment so navigator.permissions
        // and getCurrentPosition / Notification.requestPermission etc. complete
        // instead of silently denying. Camera/microphone/bluetooth still need
        // explicit user consent; we deny those for now.
        match request.feature() {
            PermissionFeature::Geolocation
            | PermissionFeature::Notifications
            | PermissionFeature::Push
            | PermissionFeature::PersistentStorage
            | PermissionFeature::BackgroundSync
            | PermissionFeature::DeviceInfo => request.allow(),
            PermissionFeature::Camera
            | PermissionFeature::Microphone
            | PermissionFeature::Speaker
            | PermissionFeature::Bluetooth
            | PermissionFeature::Midi => request.deny(),
        }
    }

    fn show_notification(&self, _webview: WebView, notification: Notification) {
        // Win32 toast (proper Action Center notification) needs WinRT XML —
        // for now surface as a non-modal MessageBox-style popup so it's at
        // least visible. A toast wrapper is a follow-up.
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::HWND;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONINFORMATION, MB_OK,
            };
            let title: Vec<u16> = notification
                .title
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let body: Vec<u16> = notification
                .body
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // HWND_DESKTOP so the message box is non-blocking on the parent and the page.
            unsafe {
                MessageBoxW(
                    self.state.parent_hwnd as HWND,
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        #[cfg(not(windows))]
        {
            let _ = notification;
        }
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: ServoCursor) {
        let mapped = map_servo_cursor(cursor);
        let _ = self.state.proxy.send_event(UserEvent::SetCursor(mapped));
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        match status {
            LoadStatus::Started => {
                let _ = self.state.proxy.send_event(UserEvent::LoadStart);
            }
            LoadStatus::Complete => {
                let _ = self.state.proxy.send_event(UserEvent::LoadEnd);
                if let Some(url) = webview.url() {
                    let url_s = url.to_string();
                    // Rewrite loopback http://127.0.0.1:<port>/<path> back to
                    // aurora://<path> for the toolbar so the user sees the
                    // friendly internal URL.
                    let display_url = match self.state.aurora_origin.as_deref() {
                        Some(origin) if url_s.starts_with(origin) => {
                            let rest = url_s[origin.len()..]
                                .trim_start_matches('/')
                                .trim_end_matches('/');
                            let path = if rest.is_empty() { "newtab" } else { rest };
                            format!("aurora://{}", path)
                        }
                        _ => url_s.clone(),
                    };
                    let _ = self
                        .state
                        .proxy
                        .send_event(UserEvent::UpdateUrl(display_url));
                    // Push history (helper skips aurora:// and data: URLs internally)
                    let title = webview.page_title().unwrap_or_default();
                    crate::history::push_history_entry(&title, &url_s);
                }
            }
            _ => {}
        }
    }

    fn show_embedder_control(&self, _webview: WebView, control: EmbedderControl) {
        match control {
            EmbedderControl::FilePicker(mut picker) => {
                #[cfg(windows)]
                {
                    // Servo's FilterPattern is a single bare extension ("png", "jpg", etc).
                    // Group them all into one filter row so the user sees "Allowed types".
                    let exts: Vec<String> =
                        picker.filter_patterns().iter().map(|p| p.0.clone()).collect();
                    let filters = if exts.is_empty() {
                        vec![]
                    } else {
                        vec![crate::dialogs::FileFilter {
                            description: format!("Allowed ({})", exts.join(", ")),
                            extensions: exts,
                        }]
                    };
                    let multi = picker.allow_select_multiple();
                    match crate::dialogs::open_file_dialog(self.state.parent_hwnd, multi, &filters)
                    {
                        Some(paths) => {
                            picker.select(&paths);
                            picker.submit();
                        }
                        None => picker.dismiss(),
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = picker;
                }
            }
            EmbedderControl::SelectElement(mut select) => {
                #[cfg(windows)]
                {
                    // Build a Win32 popup menu mirroring the <select>'s <option>/<optgroup>s.
                    let selected: std::collections::HashSet<usize> =
                        select.selected_options().into_iter().collect();
                    let items: Vec<crate::dialogs::PopupItem> = select
                        .options()
                        .iter()
                        .map(|opt_or_grp| match opt_or_grp {
                            SelectElementOptionOrOptgroup::Option(o) => {
                                crate::dialogs::PopupItem::Item {
                                    id: o.id as u32,
                                    label: o.label.clone(),
                                    checked: selected.contains(&o.id),
                                    disabled: o.is_disabled,
                                }
                            }
                            SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                                crate::dialogs::PopupItem::Group {
                                    label: label.clone(),
                                    items: options
                                        .iter()
                                        .map(|o| crate::dialogs::PopupItem::Item {
                                            id: o.id as u32,
                                            label: o.label.clone(),
                                            checked: selected.contains(&o.id),
                                            disabled: o.is_disabled,
                                        })
                                        .collect(),
                                }
                            }
                        })
                        .collect();

                    // Anchor at the bottom-left corner of the <select>, in screen coords.
                    // Servo gives device pixels relative to the webview.
                    let pos = select.position();
                    let (sx, sy) = self.client_to_screen(pos.min.x, pos.max.y);

                    match crate::dialogs::popup_menu(self.state.parent_hwnd, sx, sy, &items) {
                        Some(id) => {
                            select.select(vec![id as usize]);
                            select.submit();
                        }
                        None => select.submit(), // dismissed -> keep prior selection
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = select;
                }
            }
            EmbedderControl::ColorPicker(mut picker) => {
                #[cfg(windows)]
                {
                    let initial = picker
                        .current_color()
                        .map(|c| (c.red, c.green, c.blue));
                    let chosen = crate::dialogs::pick_color(self.state.parent_hwnd, initial)
                        .map(|(r, g, b)| servo::RgbColor { red: r, green: g, blue: b });
                    picker.select(chosen);
                    picker.submit();
                }
                #[cfg(not(windows))]
                {
                    let _ = picker;
                }
            }
            EmbedderControl::SimpleDialog(dialog) => {
                #[cfg(windows)]
                {
                    self.handle_simple_dialog(dialog);
                }
                #[cfg(not(windows))]
                {
                    let _ = dialog;
                }
            }
            EmbedderControl::ContextMenu(menu) => {
                #[cfg(windows)]
                {
                    // Map Servo's context-menu items to our generic PopupItem,
                    // remembering each Item's id -> action for the response.
                    let mut actions: Vec<ContextMenuAction> = Vec::new();
                    let items: Vec<crate::dialogs::PopupItem> = menu
                        .items()
                        .iter()
                        .map(|it| match it {
                            ServoContextMenuItem::Item {
                                label,
                                action,
                                enabled,
                            } => {
                                let id = actions.len() as u32;
                                actions.push(*action);
                                crate::dialogs::PopupItem::Item {
                                    id,
                                    label: label.clone(),
                                    checked: false,
                                    disabled: !*enabled,
                                }
                            }
                            ServoContextMenuItem::Separator => crate::dialogs::PopupItem::Separator,
                        })
                        .collect();

                    let pos = menu.position();
                    let (sx, sy) = self.client_to_screen(pos.min.x, pos.max.y);

                    match crate::dialogs::popup_menu(self.state.parent_hwnd, sx, sy, &items) {
                        Some(id) if (id as usize) < actions.len() => {
                            menu.select(actions[id as usize]);
                        }
                        _ => menu.dismiss(),
                    }
                }
                #[cfg(not(windows))]
                {
                    menu.dismiss();
                }
            }
            // InputMethod (IME for CJK input) still pending — large platform-specific
            // wire-up. Tracked in project_servo_roadmap.md.
            _ => {}
        }
    }

    fn request_navigation(&self, _webview: WebView, request: NavigationRequest) {
        let url_str = request.url.as_str();
        // Intercept aurora-ipc: scheme — decode and forward as UserEvent
        if let Some(encoded) = url_str.strip_prefix("aurora-ipc:") {
            let decoded = percent_decode(encoded);
            slog(&format!("aurora-ipc intercepted: {decoded}"));
            let _ = self.state.proxy.send_event(UserEvent::AuroraIpc(decoded));
            request.deny();
            return;
        }
        // Intercept aurora:// scheme — forward as Navigate
        if url_str.starts_with("aurora://") {
            let _ = self.state.proxy.send_event(UserEvent::Navigate(url_str.to_string()));
            request.deny();
            return;
        }
        // Allow all other navigation
        request.allow();
    }
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<u8> = s.bytes().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i+1]), hex_val(bytes[i+2])) {
                out.push((h << 4 | l) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Counts notify_new_frame_ready calls so we only log the first few.
static FRAME_LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Override the cursor of the Servo child HWND's window class so the OS
/// stops resetting it to the STATIC class default whenever the mouse enters
/// the child. Without this, tao's set_cursor_icon on the parent gets
/// overridden the moment Win32 sends WM_SETCURSOR to the child.
#[cfg(windows)]
pub fn set_child_window_cursor(child_hwnd: isize, cursor: Option<tao::window::CursorIcon>) {
    use std::ptr;
    use tao::window::CursorIcon;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LoadCursorW, SetClassLongPtrW, SetCursor, GCLP_HCURSOR, IDC_APPSTARTING, IDC_ARROW,
        IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS,
        IDC_SIZENWSE, IDC_SIZEWE, IDC_WAIT,
    };

    // Map tao::CursorIcon to a Win32 IDC_* system cursor id.
    // Win32 only ships ~14 system cursors; many CSS cursors collapse to similar ones.
    let idc: *const u16 = match cursor {
        None => return, // hidden cursor handled at the parent-window level
        Some(c) => match c {
            CursorIcon::Default | CursorIcon::Arrow => IDC_ARROW,
            CursorIcon::Hand | CursorIcon::Grab | CursorIcon::Grabbing => IDC_HAND,
            CursorIcon::Text | CursorIcon::VerticalText => IDC_IBEAM,
            CursorIcon::Crosshair | CursorIcon::Cell => IDC_CROSS,
            CursorIcon::Wait => IDC_WAIT,
            CursorIcon::Progress => IDC_APPSTARTING,
            CursorIcon::Help => IDC_HELP,
            CursorIcon::NotAllowed | CursorIcon::NoDrop => IDC_NO,
            CursorIcon::Move | CursorIcon::AllScroll => IDC_SIZEALL,
            CursorIcon::EResize
            | CursorIcon::WResize
            | CursorIcon::EwResize
            | CursorIcon::ColResize => IDC_SIZEWE,
            CursorIcon::NResize
            | CursorIcon::SResize
            | CursorIcon::NsResize
            | CursorIcon::RowResize => IDC_SIZENS,
            CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => IDC_SIZENESW,
            CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => IDC_SIZENWSE,
            // Best-effort mappings for cursors with no direct Win32 equivalent.
            CursorIcon::Alias | CursorIcon::Copy | CursorIcon::ContextMenu => IDC_ARROW,
            CursorIcon::ZoomIn | CursorIcon::ZoomOut => IDC_CROSS,
            _ => IDC_ARROW,
        },
    };

    unsafe {
        let hcursor = LoadCursorW(ptr::null_mut(), idc);
        if hcursor.is_null() {
            return;
        }
        // Persist on the window class so subsequent WM_SETCURSOR on the child returns this.
        SetClassLongPtrW(child_hwnd as HWND, GCLP_HCURSOR, hcursor as isize);
        // Also force-set right now, since the mouse may already be inside the child.
        SetCursor(hcursor);
    }
}

/// Map Servo's CSS-derived cursor to tao's window CursorIcon, which tao forwards
/// to the platform (Win32 SetCursor / Cocoa NSCursor / GTK). Returns None to
/// request a hidden cursor (Cursor::None).
fn map_servo_cursor(cursor: ServoCursor) -> Option<tao::window::CursorIcon> {
    use tao::window::CursorIcon;
    Some(match cursor {
        ServoCursor::None => return None,
        ServoCursor::Default => CursorIcon::Default,
        ServoCursor::Pointer => CursorIcon::Hand,
        ServoCursor::ContextMenu => CursorIcon::ContextMenu,
        ServoCursor::Help => CursorIcon::Help,
        ServoCursor::Progress => CursorIcon::Progress,
        ServoCursor::Wait => CursorIcon::Wait,
        ServoCursor::Cell => CursorIcon::Cell,
        ServoCursor::Crosshair => CursorIcon::Crosshair,
        ServoCursor::Text => CursorIcon::Text,
        ServoCursor::VerticalText => CursorIcon::VerticalText,
        ServoCursor::Alias => CursorIcon::Alias,
        ServoCursor::Copy => CursorIcon::Copy,
        ServoCursor::Move => CursorIcon::Move,
        ServoCursor::NoDrop => CursorIcon::NoDrop,
        ServoCursor::NotAllowed => CursorIcon::NotAllowed,
        ServoCursor::Grab => CursorIcon::Grab,
        ServoCursor::Grabbing => CursorIcon::Grabbing,
        ServoCursor::EResize => CursorIcon::EResize,
        ServoCursor::NResize => CursorIcon::NResize,
        ServoCursor::NeResize => CursorIcon::NeResize,
        ServoCursor::NwResize => CursorIcon::NwResize,
        ServoCursor::SResize => CursorIcon::SResize,
        ServoCursor::SeResize => CursorIcon::SeResize,
        ServoCursor::SwResize => CursorIcon::SwResize,
        ServoCursor::WResize => CursorIcon::WResize,
        ServoCursor::EwResize => CursorIcon::EwResize,
        ServoCursor::NsResize => CursorIcon::NsResize,
        ServoCursor::NeswResize => CursorIcon::NeswResize,
        ServoCursor::NwseResize => CursorIcon::NwseResize,
        ServoCursor::ColResize => CursorIcon::ColResize,
        ServoCursor::RowResize => CursorIcon::RowResize,
        ServoCursor::AllScroll => CursorIcon::AllScroll,
        ServoCursor::ZoomIn => CursorIcon::ZoomIn,
        ServoCursor::ZoomOut => CursorIcon::ZoomOut,
    })
}

fn slog(msg: &str) {
    use std::io::Write;
    let path = crate::config::exe_dir().join("servo_log.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[servo] {msg}");
    }
}

// ── EventLoopWaker ────────────────────────────────────────────────────────

struct TaoWaker(Arc<Mutex<EventLoopProxy<UserEvent>>>);

impl EventLoopWaker for TaoWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(TaoWaker(Arc::clone(&self.0)))
    }
    fn wake(&self) {
        let _ = self.0.lock().unwrap().send_event(UserEvent::ServoWake);
    }
}

// ── ServoView ─────────────────────────────────────────────────────────────

/// Embedded Servo instance — renders web content into a child HWND below Aurora's toolbar.
pub struct ServoView {
    pub servo: servo::Servo,
    pub webview: servo::WebView,
    pub rendering_context: Rc<WindowRenderingContext>,
    pub toolbar_height_phys: u32,
    child_hwnd: isize,
    parent_hwnd: isize,
    state: Rc<ServoState>,
}

impl ServoView {
    /// Create a Servo instance for the content area.
    /// `win_width` / `win_height` are the **physical** pixel dimensions of the tao window.
    /// `toolbar_height_phys` is the **physical** pixel height of the toolbar area.
    pub fn new(
        window: &Window,
        proxy: EventLoopProxy<UserEvent>,
        win_width: u32,
        win_height: u32,
        toolbar_height_phys: u32,
        initial_url: &str,
        scale_factor: f64,
        aurora_origin: Option<String>,
    ) -> Result<Self, String> {
        // Get the parent HWND from tao
        let parent_hwnd = {
            let wh = window
                .window_handle()
                .map_err(|e| format!("window_handle: {e}"))?;
            match wh.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => return Err("Not a Win32 window".to_string()),
            }
        };

        let content_h = win_height.saturating_sub(toolbar_height_phys).max(1);

        // Create child HWND positioned below the toolbar
        let child_hwnd = child_window::create(
            parent_hwnd,
            0,
            toolbar_height_phys as i32,
            win_width as i32,
            content_h as i32,
        );
        if child_hwnd == 0 {
            return Err("Failed to create child HWND".to_string());
        }

        let raw_hwnd = RawHwnd(child_hwnd);
        let size = PhysicalSize::new(win_width.max(1), content_h);

        let rendering_context = Rc::new(
            WindowRenderingContext::new(
                raw_hwnd.display_handle().unwrap(),
                raw_hwnd.window_handle().unwrap(),
                size,
            )
            .map_err(|e| format!("WindowRenderingContext: {e:?}"))?,
        );

        let _ = rendering_context.make_current();

        let state = Rc::new(ServoState {
            needs_paint: Cell::new(false),
            proxy: proxy.clone(),
            parent_hwnd,
            aurora_origin,
        });

        let waker: Box<dyn EventLoopWaker> = Box::new(TaoWaker(Arc::new(Mutex::new(proxy))));

        // Turn on Web platform features that Servo gates behind preferences.
        // Each one is a small but visible win in aurora://test:
        // - dom_notification_enabled       -> window.Notification constructor
        // - dom_intersection_observer_enabled -> IntersectionObserver
        // - dom_indexeddb_enabled          -> indexedDB.open() works
        // - dom_serviceworker_enabled      -> navigator.serviceWorker
        // - dom_async_clipboard_enabled    -> navigator.clipboard.writeText/readText
        // - dom_geolocation_enabled        -> navigator.geolocation
        // - dom_offscreen_canvas_enabled   -> OffscreenCanvas (cheap to enable)
        let mut prefs = servo::Preferences::default();
        prefs.dom_notification_enabled = true;
        prefs.dom_intersection_observer_enabled = true;
        prefs.dom_indexeddb_enabled = true;
        prefs.dom_serviceworker_enabled = true;
        prefs.dom_async_clipboard_enabled = true;
        prefs.dom_geolocation_enabled = true;
        prefs.dom_offscreen_canvas_enabled = true;
        prefs.dom_webgl2_enabled = true;
        prefs.dom_navigator_protocol_handlers_enabled = true;

        let servo = ServoBuilder::default()
            .event_loop_waker(waker)
            .preferences(prefs)
            .build();

        let url = ServoUrl::parse(initial_url)
            .unwrap_or_else(|_| ServoUrl::parse("https://www.google.com").unwrap());

        let delegate = Rc::new(AuroraDelegate {
            state: Rc::clone(&state),
            child_hwnd,
            toolbar_phys: toolbar_height_phys,
        });

        let hidpi = Scale::<f32, DeviceIndependentPixel, _>::new(scale_factor as f32);

        let webview = WebViewBuilder::new(
            &servo,
            rendering_context.clone() as Rc<dyn RenderingContext>,
        )
        .url(url.into_url())
        .delegate(delegate)
        .hidpi_scale_factor(hidpi)
        .build();

        webview.focus();
        webview.show();

        slog(&format!("ServoView initialized — child_hwnd={child_hwnd}, size={win_width}x{win_height}, toolbar_h={toolbar_height_phys}, dpr={scale_factor}"));

        Ok(Self {
            servo,
            webview,
            rendering_context,
            toolbar_height_phys,
            child_hwnd,
            parent_hwnd,
            state,
        })
    }

    // ── Tick ──────────────────────────────────────────────────────────────

    /// Spin Servo's event loop once (non-blocking). Call when woken by ServoWake.
    pub fn spin(&self) {
        self.servo.spin_event_loop();
    }

    /// Paint and present only when Servo says a new frame is ready.
    /// Returns true if something was painted.
    pub fn paint_if_needed(&self) -> bool {
        if self.state.needs_paint.get() {
            self.state.needs_paint.set(false);
            let mc = self.rendering_context.make_current();
            if mc.is_err() { slog(&format!("make_current FAILED: {:?}", mc)); }
            self.webview.paint();
            self.rendering_context.present();
            self.bring_to_front();
            true
        } else {
            false
        }
    }

    /// Order child HWNDs: Servo below toolbar, toolbar on top.
    /// Strategy: put Servo at HWND_TOP first, then put all other child windows
    /// (wry toolbar) above Servo so toolbar remains visible.
    fn bring_to_front(&self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::HWND;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindow, SetWindowPos,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, GW_CHILD, GW_HWNDNEXT,
            };

            // First put Servo at bottom of Z-order (HWND_BOTTOM = 1).
            const HWND_BOTTOM: HWND = 1isize as _;
            SetWindowPos(
                self.child_hwnd as HWND,
                HWND_BOTTOM,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );

            // Then raise all wry child windows (not our Servo HWND) to top.
            // Walk sibling HWNDs and push non-Servo ones to HWND_TOP.
            const HWND_TOP: HWND = 0isize as _;
            let mut sibling = GetWindow(self.parent_hwnd as HWND, GW_CHILD);
            let mut count = 0;
            while sibling != 0 as _ {
                count += 1;
                if sibling as isize != self.child_hwnd {
                    SetWindowPos(
                        sibling,
                        HWND_TOP,
                        0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
                sibling = GetWindow(sibling, GW_HWNDNEXT);
            }
            // Log only on first few calls
            static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                slog(&format!("bring_to_front: child count={count}, servo_hwnd={}", self.child_hwnd));
            }
        }
    }

    /// Returns the Win32 child HWND that hosts the Servo render surface.
    /// Used by the embedder to override the child window's class cursor so
    /// CSS cursor changes survive WM_SETCURSOR routing.
    pub fn child_hwnd(&self) -> isize {
        self.child_hwnd
    }

    /// Force-exit the process immediately. Used on window close to avoid
    /// hanging in ServoInner::drop() which spins the event loop synchronously.
    pub fn force_exit() -> ! {
        std::process::exit(0);
    }

    // ── Resize ────────────────────────────────────────────────────────────

    pub fn resize(&self, win_width: u32, win_height: u32) {
        let content_h = win_height.saturating_sub(self.toolbar_height_phys).max(1);
        child_window::set_bounds(
            self.child_hwnd,
            0,
            self.toolbar_height_phys as i32,
            win_width as i32,
            content_h as i32,
        );
        self.webview
            .resize(PhysicalSize::new(win_width.max(1), content_h));
    }

    pub fn set_scale_factor(&self, scale_factor: f64) {
        let hidpi = Scale::<f32, DeviceIndependentPixel, _>::new(scale_factor as f32);
        self.webview.set_hidpi_scale_factor(hidpi);
    }

    // ── Navigation ────────────────────────────────────────────────────────

    pub fn navigate(&self, url: &str) {
        if let Ok(parsed) = ServoUrl::parse(url) {
            self.webview.load(parsed.into_url());
        }
    }

    /// Load raw HTML into Servo via a data: URL.
    pub fn load_html(&self, html: &str) {
        let encoded = base64_encode(html.as_bytes());
        let data_url = format!("data:text/html;base64,{}", encoded);
        if let Ok(parsed) = ServoUrl::parse(&data_url) {
            self.webview.load(parsed.into_url());
        }
    }

    pub fn go_back(&self) {
        if self.webview.can_go_back() {
            self.webview.go_back(1);
        }
    }

    pub fn go_forward(&self) {
        if self.webview.can_go_forward() {
            self.webview.go_forward(1);
        }
    }

    pub fn reload(&self) {
        self.webview.reload();
    }

    /// Run JavaScript in the active page. Result is dropped — fire-and-forget.
    pub fn run_js(&self, script: &str) {
        self.webview.evaluate_javascript(script.to_string(), |_| {});
    }

    /// Pinch-zoom by a delta around the viewport center. Servo clamps to [1.0, 10.0].
    pub fn adjust_zoom(&self, delta: f32) {
        let center = DevicePoint::new(0.0, 0.0);
        self.webview.adjust_pinch_zoom(delta, center);
    }

    /// Reset pinch-zoom to 1.0 by calling adjust with the inverse of current zoom.
    pub fn reset_zoom(&self) {
        let cur = self.webview.pinch_zoom();
        if (cur - 1.0).abs() > f32::EPSILON {
            self.adjust_zoom(1.0 / cur);
        }
    }

    // ── Input helpers ─────────────────────────────────────────────────────

    /// Convert full-window physical coords to child-window-relative coords.
    /// The child window starts at toolbar_height_phys, so subtract that from y.
    fn content_point(x: f64, y: f64, toolbar_h: u32) -> Option<WebViewPoint> {
        let wy = y - toolbar_h as f64;
        if wy < 0.0 {
            return None;
        }
        Some(WebViewPoint::Device(DevicePoint::new(x as f32, wy as f32)))
    }

    // ── Mouse ─────────────────────────────────────────────────────────────

    pub fn on_mouse_move(&self, x: f64, y: f64) {
        if let Some(point) = Self::content_point(x, y, self.toolbar_height_phys) {
            self.webview
                .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point)));
        }
    }

    pub fn on_mouse_button(&self, x: f64, y: f64, button: tao::event::MouseButton, pressed: bool) {
        if let Some(point) = Self::content_point(x, y, self.toolbar_height_phys) {
            let btn = match button {
                tao::event::MouseButton::Left => ServoMouseButton::Left,
                tao::event::MouseButton::Right => ServoMouseButton::Right,
                tao::event::MouseButton::Middle => ServoMouseButton::Middle,
                tao::event::MouseButton::Other(n) => ServoMouseButton::Other(n),
                _ => ServoMouseButton::Other(0),
            };
            let action = if pressed {
                MouseButtonAction::Down
            } else {
                MouseButtonAction::Up
            };
            self.webview
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    action, btn, point,
                )));
        }
    }

    pub fn on_scroll(&self, x: f64, y: f64, dx: f64, dy: f64) {
        if let Some(point) = Self::content_point(x, y, self.toolbar_height_phys) {
            let delta = WheelDelta {
                x: dx,
                y: dy,
                z: 0.0,
                mode: WheelMode::DeltaPixel,
            };
            self.webview
                .notify_input_event(InputEvent::Wheel(WheelEvent::new(delta, point)));
        }
    }

    // ── Keyboard ──────────────────────────────────────────────────────────

    pub fn on_key(&self, key_event: keyboard_types::KeyboardEvent) {
        self.webview
            .notify_input_event(InputEvent::Keyboard(ServoKeyboardEvent::new(key_event)));
    }

    // ── Queries ───────────────────────────────────────────────────────────

    pub fn current_url(&self) -> Option<String> {
        self.webview.url().map(|u| u.to_string())
    }

    pub fn title(&self) -> Option<String> {
        self.webview.page_title()
    }
}
