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
    DeviceIndependentPixel, DevicePoint, EventLoopWaker, InputEvent,
    KeyboardEvent as ServoKeyboardEvent, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, RenderingContext, ServoBuilder, ServoUrl, WebView,
    WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
    WindowRenderingContext,
};
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
}

struct AuroraDelegate {
    state: Rc<ServoState>,
}

impl WebViewDelegate for AuroraDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.state.needs_paint.set(true);
        let _ = self.state.proxy.send_event(UserEvent::ServoWake);
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
        });

        let waker: Box<dyn EventLoopWaker> = Box::new(TaoWaker(Arc::new(Mutex::new(proxy))));
        let servo = ServoBuilder::default().event_loop_waker(waker).build();

        let url = ServoUrl::parse(initial_url)
            .unwrap_or_else(|_| ServoUrl::parse("https://www.google.com").unwrap());

        let delegate = Rc::new(AuroraDelegate {
            state: Rc::clone(&state),
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

        Ok(Self {
            servo,
            webview,
            rendering_context,
            toolbar_height_phys,
            child_hwnd,
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
            // make_current() must be called before each paint — EGL context may have been
            // stolen by the wry toolbar WebView between frames.
            let _ = self.rendering_context.make_current();
            self.webview.paint();
            self.rendering_context.present();
            // Bring child HWND to top so it isn't covered by wry toolbar backing layer.
            self.bring_to_front();
            true
        } else {
            false
        }
    }

    /// Raise the child HWND above other child windows so Servo content is visible.
    fn bring_to_front(&self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
            };
            // HWND_TOP = 0 — places window at top of Z-order among siblings.
            const HWND_TOP: windows_sys::Win32::Foundation::HWND = 0isize as _;
            SetWindowPos(
                self.child_hwnd as windows_sys::Win32::Foundation::HWND,
                HWND_TOP,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
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
