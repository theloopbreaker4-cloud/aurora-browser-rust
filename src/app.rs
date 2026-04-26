// app.rs — event loop, window creation, UserEvent dispatch
use crate::about;
use crate::bookmarks_page;
use crate::downloads_page;
use crate::error;
use crate::events::UserEvent;
use crate::history;
use crate::ipc::generate_ipc_token;
use crate::settings;
use crate::webviews;
use std::process::Command;
use std::time::Instant;
use tao::dpi::PhysicalPosition;
use tao::event::{ElementState, Event, MouseButton, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{CursorIcon, Fullscreen, WindowBuilder};

// Total height of the toolbar area in logical pixels (tab bar + nav bar + bookmark bar)
const TOOLBAR_HEIGHT: u32 = 122;

fn flog(msg: &str) {
    use std::io::Write;
    let path = crate::config::exe_dir().join("servo_log.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[app] {msg}");
    }
}

pub fn run() {
    // Check for --engine=servo/webview2 CLI arg (set by SwitchEngine restart).
    // This overrides config.json to avoid race conditions on write/read.
    let engine_arg: Option<String> = std::env::args()
        .find(|a| a.starts_with("--engine="))
        .map(|a| a["--engine=".len()..].to_string());

    // If passed on CLI, persist it so future cold starts use the right engine
    if let Some(ref e) = engine_arg {
        crate::config::set_engine(e);
    }

    let active_engine = engine_arg.unwrap_or_else(|| crate::config::get_engine());
    #[cfg(not(feature = "servo-engine"))]
    let _ = active_engine;
    #[cfg(feature = "servo-engine")]
    let use_servo = active_engine == "servo";

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_token = generate_ipc_token();

    // Internal HTTP server on 127.0.0.1: serves aurora:// internal pages over a
    // real http origin so localStorage / IndexedDB / SecureContext APIs work
    // (data: URLs always have an opaque origin, which the spec disallows).
    // Used only by Servo; wry continues to receive HTML via load_html.
    let internal_server_port = match crate::internal_server::start_with_default_routes(&ipc_token) {
        Ok(s) => Some(s.port),
        Err(_) => None,
    };
    let aurora_origin =
        internal_server_port.map(|p| format!("http://127.0.0.1:{}", p));

    // Build the main window
    let mut wb = WindowBuilder::new()
        .with_title("Aurora Browser")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .with_decorations(false);
    if let Some(ico) = crate::icon::load_aurora_icon() {
        wb = wb.with_window_icon(Some(ico));
    }
    let window = wb.build(&event_loop).expect("Failed to create window");

    // Paint Win32 window background black so no white flash appears before Servo renders.
    #[cfg(all(windows, feature = "servo-engine"))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetClassLongPtrW, GCLP_HBRBACKGROUND};
        use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH};
        if let Ok(wh) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = wh.as_raw() {
                unsafe {
                    let black_brush = GetStockObject(BLACK_BRUSH as i32);
                    SetClassLongPtrW(h.hwnd.get() as _, GCLP_HBRBACKGROUND, black_brush as isize);
                }
            }
        }
    }

    let size = window.inner_size().to_logical::<u32>(window.scale_factor());
    let toolbar_bounds = webviews::toolbar_bounds(size.width, TOOLBAR_HEIGHT);

    // Toolbar WebView is always wry (Aurora UI chrome)
    let toolbar_webview = webviews::build_toolbar_webview(&window, toolbar_bounds, proxy.clone());

    // ── Content area: wry or Servo ────────────────────────────────────────
    let content_bounds = webviews::content_bounds(size.width, size.height, TOOLBAR_HEIGHT);

    // Find first arg that is not a --flag (i.e. a URL)
    let startup_url = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .or_else(|| crate::config::get_last_url())
        .unwrap_or_else(|| "aurora://newtab".to_string());

    // Servo embedded view (only when servo-engine feature + engine=servo config)
    #[cfg(feature = "servo-engine")]
    let servo_view: Option<crate::servo_view::ServoView> = if use_servo {
        let phys = window.inner_size();
        let dpr = window.scale_factor();
        let toolbar_phys = (TOOLBAR_HEIGHT as f64 * dpr) as u32;
        // For aurora:// startup URLs, route through the loopback HTTP server
        // (real http origin so localStorage etc work). Falls back to about:blank
        // if the server failed to start; the load_html path below will take over.
        let url = if startup_url.starts_with("aurora://") {
            if let Some(ref origin) = aurora_origin {
                let path = startup_url.strip_prefix("aurora://").unwrap_or("newtab");
                let canonical = if path.is_empty() || path == "portal" { "newtab" } else { path };
                format!("{}/{}", origin, canonical)
            } else {
                "about:blank".to_string()
            }
        } else {
            startup_url.clone()
        };
        match crate::servo_view::ServoView::new(
            &window,
            proxy.clone(),
            phys.width,
            phys.height,
            toolbar_phys,
            &url,
            dpr,
            aurora_origin.clone(),
        ) {
            Ok(sv) => {
                if startup_url.starts_with("aurora://") {
                    if aurora_origin.is_none() {
                        // No internal server -> fall back to embedding via load_html.
                        sv.load_html(&webviews::portal_html(&ipc_token));
                    }
                    // Toolbar should display the aurora:// URL even if Servo loaded the http alias.
                    let _ = proxy.send_event(UserEvent::UpdateUrl("aurora://newtab".to_string()));
                } else {
                    let _ = proxy.send_event(UserEvent::UpdateUrl(url.clone()));
                }
                Some(sv)
            }
            Err(e) => {
                eprintln!("Servo init failed: {e}. Falling back to webview2.");
                None
            }
        }
    } else {
        None
    };

    // wry content WebView — only built when NOT using servo
    let content_webview_opt = if {
        #[cfg(feature = "servo-engine")]
        {
            !use_servo || servo_view.is_none()
        }
        #[cfg(not(feature = "servo-engine"))]
        {
            true
        }
    } {
        let portal_html = webviews::portal_html(&ipc_token);
        let cv = webviews::build_content_webview(
            &window,
            content_bounds,
            proxy.clone(),
            &ipc_token,
            &portal_html,
        );
        if !startup_url.starts_with("aurora://") {
            let _ = cv.load_url(&startup_url);
            let _ = proxy.send_event(UserEvent::UpdateUrl(startup_url.clone()));
        } else {
            let _ = proxy.send_event(UserEvent::UpdateUrl("aurora://newtab".to_string()));
        }
        Some(cv)
    } else {
        None
    };

    // Pre-generate portal HTML for newtab navigation during event loop
    let portal_html_for_loop = webviews::portal_html(&ipc_token);
    let proxy_for_events = proxy.clone();

    // Notify toolbar which engine is active
    #[cfg(feature = "servo-engine")]
    let engine_label = if servo_view.is_some() {
        "servo"
    } else {
        "webview2"
    };
    #[cfg(not(feature = "servo-engine"))]
    let engine_label = "webview2";
    let _ = proxy.send_event(UserEvent::UpdateUrl(startup_url.clone()));
    let _ = toolbar_webview.evaluate_script(&format!(
        "typeof onEngineChanged === 'function' && onEngineChanged('{}')",
        engine_label
    ));

    // For Servo: use Poll mode for the first N events so Servo can process its
    // internal startup queue and paint the first frame quickly.
    #[cfg(feature = "servo-engine")]
    let mut servo_warmup_ticks: u32 = if servo_view.is_some() { 60 } else { 0 };

    // Track cursor position for edge resize detection
    let mut cursor_pos: PhysicalPosition<f64> = PhysicalPosition::new(0.0, 0.0);
    const RESIZE_BORDER: f64 = 6.0;
    let mut last_lclick: Option<Instant> = None;
    const DBLCLICK_MS: u128 = 400;
    let mut current_url = if startup_url.starts_with("aurora://") {
        "aurora://newtab".to_string()
    } else {
        startup_url.clone()
    };

    event_loop.run(move |event, _, control_flow| {
        // Default: sleep until next event. Servo wakes us via TaoWaker when it needs work.
        *control_flow = ControlFlow::Wait;

        // During Servo startup, use Poll so we keep spinning until first frame is painted.
        #[cfg(feature = "servo-engine")]
        if servo_warmup_ticks > 0 {
            servo_warmup_ticks -= 1;
            *control_flow = ControlFlow::Poll;
            // Spin during warmup on every tick so Servo processes its startup queue.
            if let Some(ref sv) = servo_view {
                sv.spin();
            }
        }

        // Servo: paint only when Servo signals a new frame is ready.
        // spin() is called in ServoWake handler (not on every event) to avoid stalling.
        #[cfg(feature = "servo-engine")]
        if let Some(ref sv) = servo_view {
            let painted = sv.paint_if_needed();
            // Once first frame arrives, stop warmup polling.
            #[allow(unused_assignments)]
            if painted && servo_warmup_ticks > 0 {
                servo_warmup_ticks = 0;
            }
            // Sync URL/title after a paint (new frame = possible navigation)
            if painted {
                if let Some(url) = sv.current_url() {
                    if url != current_url {
                        current_url = url.clone();
                        let escaped = url.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = toolbar_webview.evaluate_script(&format!("onUrlChanged('{}')", escaped));
                    }
                }
                if let Some(title) = sv.title() {
                    let escaped = title.replace('\\', "\\\\").replace('\'', "\\'");
                    let _ = toolbar_webview.evaluate_script(&format!("onTitleChanged('{}')", escaped));
                }
            }
        }

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested, ..
            } => {
                // When Servo is active, ServoInner::drop() blocks spinning the event loop
                // which deadlocks since the event loop has already exited. Force-exit instead.
                #[cfg(feature = "servo-engine")]
                if servo_view.is_some() {
                    crate::servo_view::ServoView::force_exit();
                }
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::Resized(new_size), ..
            } => {
                let s = new_size.to_logical::<u32>(window.scale_factor());
                let _ = toolbar_webview
                    .set_bounds(webviews::toolbar_bounds(s.width, TOOLBAR_HEIGHT));
                if let Some(ref cv) = content_webview_opt {
                    let _ = cv.set_bounds(webviews::content_bounds(s.width, s.height, TOOLBAR_HEIGHT));
                }
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    sv.resize(new_size.width, new_size.height);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                cursor_pos = position;
                // Poke Servo with mouse move
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    sv.on_mouse_move(position.x, position.y);
                }
                // Edge resize cursor icons
                let size = window.inner_size();
                let (w, h) = (size.width as f64, size.height as f64);
                let (x, y) = (position.x, position.y);
                let cursor = match (x < RESIZE_BORDER, x > w - RESIZE_BORDER, y < RESIZE_BORDER, y > h - RESIZE_BORDER) {
                    (true,  false, true,  false) => CursorIcon::NwResize,
                    (false, true,  true,  false) => CursorIcon::NeResize,
                    (true,  false, false, true)  => CursorIcon::SwResize,
                    (false, true,  false, true)  => CursorIcon::SeResize,
                    (true,  false, false, false) => CursorIcon::WResize,
                    (false, true,  false, false) => CursorIcon::EResize,
                    (false, false, true,  false) => CursorIcon::NResize,
                    (false, false, false, true)  => CursorIcon::SResize,
                    _ => CursorIcon::Default,
                };
                window.set_cursor_icon(cursor);
            }

            Event::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. }, ..
            } => {
                let pressed = state == ElementState::Pressed;
                let y = cursor_pos.y;

                // Forward to Servo if below toolbar
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    sv.on_mouse_button(cursor_pos.x, y, button, pressed);
                }

                // Double-click titlebar (top 42px)
                if y < 42.0 && button == MouseButton::Left && pressed {
                    let now = Instant::now();
                    let is_dbl = last_lclick
                        .map(|t| now.duration_since(t).as_millis() < DBLCLICK_MS)
                        .unwrap_or(false);
                    if is_dbl {
                        window.set_maximized(!window.is_maximized());
                        last_lclick = None;
                    } else {
                        last_lclick = Some(now);
                    }
                }
            }

            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta: _delta, .. }, ..
            } => {
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    use tao::event::MouseScrollDelta;
                    let (dx, dy) = match _delta {
                        MouseScrollDelta::LineDelta(x, y) => (x as f64 * 40.0, y as f64 * 40.0),
                        MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                        _ => (0.0, 0.0),
                    };
                    sv.on_scroll(cursor_pos.x, cursor_pos.y, dx, dy);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event: _key_event, .. }, ..
            } => {
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    use tao::keyboard::Key as TaoKey;
                    use keyboard_types::{Key, KeyState, KeyboardEvent as KbEvent, Code, Location, Modifiers};
                    let state = match _key_event.state {
                        ElementState::Pressed  => KeyState::Down,
                        ElementState::Released => KeyState::Up,
                        _ => KeyState::Down,
                    };
                    // Map tao Key to keyboard_types Key (best-effort)
                    let key = match &_key_event.logical_key {
                        TaoKey::Character(c) => Key::Character(c.to_string()),
                        TaoKey::Enter => Key::Named(keyboard_types::NamedKey::Enter),
                        TaoKey::Backspace => Key::Named(keyboard_types::NamedKey::Backspace),
                        TaoKey::Escape => Key::Named(keyboard_types::NamedKey::Escape),
                        TaoKey::Tab => Key::Named(keyboard_types::NamedKey::Tab),
                        TaoKey::Space => Key::Character(" ".to_string()),
                        TaoKey::ArrowLeft  => Key::Named(keyboard_types::NamedKey::ArrowLeft),
                        TaoKey::ArrowRight => Key::Named(keyboard_types::NamedKey::ArrowRight),
                        TaoKey::ArrowUp    => Key::Named(keyboard_types::NamedKey::ArrowUp),
                        TaoKey::ArrowDown  => Key::Named(keyboard_types::NamedKey::ArrowDown),
                        TaoKey::Home  => Key::Named(keyboard_types::NamedKey::Home),
                        TaoKey::End   => Key::Named(keyboard_types::NamedKey::End),
                        TaoKey::PageUp   => Key::Named(keyboard_types::NamedKey::PageUp),
                        TaoKey::PageDown => Key::Named(keyboard_types::NamedKey::PageDown),
                        TaoKey::Delete   => Key::Named(keyboard_types::NamedKey::Delete),
                        _ => Key::Named(keyboard_types::NamedKey::Unidentified),
                    };
                    let kb = KbEvent {
                        state,
                        key,
                        code: Code::Unidentified,
                        location: Location::Standard,
                        modifiers: Modifiers::empty(),
                        repeat: _key_event.repeat,
                        is_composing: false,
                    };
                    sv.on_key(kb);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::Focused(focused), ..
            } => {
                let _ = toolbar_webview.evaluate_script(
                    &format!("document.body.setAttribute('data-focused', '{}')", focused)
                );
            }

            Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { new_inner_size, scale_factor: new_scale, .. }, ..
            } => {
                let s = new_inner_size.to_logical::<u32>(window.scale_factor());
                let _ = toolbar_webview
                    .set_bounds(webviews::toolbar_bounds(s.width, TOOLBAR_HEIGHT));
                if let Some(ref cv) = content_webview_opt {
                    let _ = cv.set_bounds(webviews::content_bounds(s.width, s.height, TOOLBAR_HEIGHT));
                }
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    sv.set_scale_factor(new_scale);
                    sv.resize(new_inner_size.width, new_inner_size.height);
                }
            }

            Event::UserEvent(ref user_event) => {
                // ServoWake: Servo has work to do — spin its event loop now.
                #[cfg(feature = "servo-engine")]
                {
                    if let UserEvent::ServoWake = user_event {
                        if let Some(ref sv) = servo_view {
                            sv.spin();
                        }
                        return;
                    }
                }

                // AuroraIpc: IPC message from aurora:// page running inside Servo
                #[cfg(feature = "servo-engine")]
                if let UserEvent::AuroraIpc(msg) = user_event {
                    // Strip IPC token prefix (format: "TOKEN:message")
                    let msg = if let Some(pos) = msg.find(':') { &msg[pos+1..] } else { msg.as_str() };
                    if let Some(engine) = msg.strip_prefix("switch_engine:") {
                        let engine = engine.to_string();
                        if !current_url.starts_with("aurora://") {
                            crate::config::set_last_url(&current_url);
                        }
                        crate::config::set_engine(&engine);
                        if let Ok(exe) = std::env::current_exe() {
                            flog(&format!("switch_engine: spawning {:?} --engine={}", exe, engine));
                            match Command::new(&exe).arg(format!("--engine={}", engine)).spawn() {
                                Ok(_) => {
                                    flog("switch_engine: spawn OK, sleeping 500ms");
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                }
                                Err(e) => flog(&format!("switch_engine: spawn FAILED: {e}")),
                            }
                        } else {
                            flog("switch_engine: current_exe() failed");
                        }
                        flog("switch_engine: calling force_exit");
                        crate::servo_view::ServoView::force_exit();
                    } else if msg == "app:restart" {
                        if let Ok(exe) = std::env::current_exe() {
                            if let Ok(_child) = Command::new(&exe).spawn() {
                                std::thread::sleep(std::time::Duration::from_millis(300));
                            }
                        }
                        crate::servo_view::ServoView::force_exit();
                    } else if let Some(rest) = msg.strip_prefix("config:") {
                        let parts: Vec<&str> = rest.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let dir = crate::config::exe_dir();
                            let config_path = dir.join("config.json");
                            let config_str = std::fs::read_to_string(&config_path)
                                .unwrap_or_else(|_| "{}".to_string());
                            let new_config = crate::config::update_config_value(&config_str, parts[0], parts[1]);
                            let _ = std::fs::write(&config_path, &new_config);
                            let _ = std::fs::write("config.json", &new_config);
                            if parts[0] == "theme" {
                                let _ = proxy_for_events
                                    .send_event(UserEvent::ApplyTheme(parts[1].to_string()));
                            }
                        }
                    }
                    return;
                }

                // When Servo is active, forward navigation/input to it
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    match user_event {
                        UserEvent::Navigate(url) if !url.starts_with("aurora://") => {
                            sv.navigate(url);
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                            return;
                        }
                        UserEvent::Navigate(url) if url.starts_with("aurora://") => {
                            // Prefer routing aurora:// internal pages through the loopback
                            // HTTP server so they get a real (tuple) http origin and have
                            // localStorage / IndexedDB / SecureContext available. Fall back
                            // to load_html (data: URL, opaque origin) only if the server
                            // failed to start.
                            let path = url.strip_prefix("aurora://").unwrap_or("");
                            let canonical = if path.is_empty() || path == "portal" {
                                "newtab".to_string()
                            } else {
                                path.to_string()
                            };
                            if let Some(ref origin) = aurora_origin {
                                sv.navigate(&format!("{}/{}", origin, canonical));
                            } else {
                                // Fallback path — same as before, just with all routes in one match.
                                let html = match canonical.as_str() {
                                    "settings" => settings::get_settings_html(&ipc_token),
                                    "history" => history::get_history_html(&ipc_token),
                                    "bookmarks" => bookmarks_page::get_bookmarks_html(&ipc_token),
                                    "downloads" => downloads_page::get_downloads_html(&ipc_token),
                                    "about" => about::get_about_html(&ipc_token),
                                    "test" => crate::test_page::get_test_html(),
                                    "extensions" => crate::extensions::get_extensions_html(&ipc_token),
                                    "incognito" => crate::incognito::get_incognito_html(&ipc_token),
                                    "tab_groups" => crate::tab_groups::get_tab_groups_html(&ipc_token),
                                    "benchmarks" => crate::benchmarks::get_benchmarks_html(&ipc_token),
                                    "feedback" => crate::feedback::get_feedback_html(&ipc_token),
                                    _ => portal_html_for_loop.clone(),
                                };
                                sv.load_html(&html);
                            }
                            let display_url = if canonical == "newtab" {
                                "aurora://newtab".to_string()
                            } else {
                                format!("aurora://{}", canonical)
                            };
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(display_url));
                            return;
                        }
                        UserEvent::GoBack    => { sv.go_back();    return; }
                        UserEvent::GoForward => { sv.go_forward(); return; }
                        UserEvent::Reload    => { sv.reload();     return; }
                        UserEvent::Stop      => { /* Servo has no public stop API yet */ return; }
                        UserEvent::Print => {
                            sv.run_js("window.print && window.print()");
                            return;
                        }
                        UserEvent::ViewSource => {
                            // Best effort: re-load current URL prefixed with view-source: scheme.
                            // Servo doesn't ship a built-in viewer, so wrap the source as text.
                            sv.run_js(
                                "(()=>{const html='<pre style=\\'white-space:pre-wrap;font-family:monospace;padding:12px\\'>'+document.documentElement.outerHTML.replace(/[<>&]/g,c=>({'<':'&lt;','>':'&gt;','&':'&amp;'}[c]))+'</pre>';document.open();document.write(html);document.close();})()"
                            );
                            return;
                        }
                        UserEvent::OpenDevTools => {
                            // No public DevTools toggle in Servo; surface a console hint instead.
                            sv.run_js("console.log('Aurora: Servo DevTools not available — use --devtools flag at startup')");
                            return;
                        }
                        UserEvent::FindText(text) => {
                            // Use window.find() if present, else select-and-scroll fallback.
                            let esc = text.replace('\\', "\\\\").replace('"', "\\\"");
                            sv.run_js(&format!(
                                "(()=>{{const t=\"{esc}\";if(!t){{window.getSelection&&window.getSelection().removeAllRanges();return;}}if(window.find){{window.find(t,false,false,true);}}}})()"
                            ));
                            return;
                        }
                        UserEvent::FindPrev(text) => {
                            let esc = text.replace('\\', "\\\\").replace('"', "\\\"");
                            sv.run_js(&format!(
                                "(()=>{{const t=\"{esc}\";if(!t)return;if(window.find){{window.find(t,false,true,true);}}}})()"
                            ));
                            return;
                        }
                        UserEvent::SetZoom(level) => {
                            // wry's zoom() takes an absolute factor; for Servo we approximate by
                            // resetting then applying the factor as a single delta.
                            sv.reset_zoom();
                            if (*level - 1.0).abs() > f64::EPSILON {
                                sv.adjust_zoom(*level as f32);
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                match user_event {
                    UserEvent::Navigate(url) => {
                        if url == "aurora://newtab" || url == "aurora://portal" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&portal_html_for_loop);
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl("aurora://newtab".to_string()));
                        } else if url == "aurora://settings" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&settings::get_settings_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://history" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&history::get_history_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://bookmarks" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&bookmarks_page::get_bookmarks_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://downloads" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&downloads_page::get_downloads_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://about" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&about::get_about_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://test" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::test_page::get_test_html());
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://extensions" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::extensions::get_extensions_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://incognito" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::incognito::get_incognito_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://tab_groups" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::tab_groups::get_tab_groups_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://benchmarks" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::benchmarks::get_benchmarks_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url == "aurora://feedback" {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::feedback::get_feedback_html(&ipc_token));
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else if url.starts_with("aurora://error") {
                            if let Some(ref cv) = content_webview_opt {
                                let html = error::get_error_html(&ipc_token, &error::ErrorInfo::default());
                                let _ = cv.load_html(&html);
                            }
                            let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                        } else {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_url(url);
                            }
                        }
                    }

                    UserEvent::GoBack => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script("history.back()");
                        }
                    }
                    UserEvent::GoForward => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script("history.forward()");
                        }
                    }
                    UserEvent::Reload => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script(&format!(
                                "if(document.title==='Aurora') window.ipc.postMessage('{}:navigate:aurora://newtab'); else location.reload();",
                                ipc_token
                            ));
                        }
                    }
                    UserEvent::Stop => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script("window.stop()");
                        }
                    }

                    UserEvent::Restart => {
                        if let Ok(exe) = std::env::current_exe() {
                            let mut cmd = Command::new(exe);
                            for arg in std::env::args().skip(1) { cmd.arg(arg); }
                            let _ = cmd.spawn();
                        }
                        *control_flow = ControlFlow::Exit;
                    }

                    UserEvent::SetCursor(cursor) => {
                        match cursor {
                            Some(icon) => {
                                window.set_cursor_visible(true);
                                window.set_cursor_icon(*icon);
                            }
                            None => {
                                window.set_cursor_visible(false);
                            }
                        }
                        // When Servo hosts the content in a Win32 child HWND, the child's
                        // window class returns its own cursor on every WM_SETCURSOR — override it.
                        #[cfg(all(windows, feature = "servo-engine"))]
                        if let Some(ref sv) = servo_view {
                            crate::servo_view::set_child_window_cursor(sv.child_hwnd(), *cursor);
                        }
                    }
                    UserEvent::ApplyTheme(theme) => {
                        let esc = theme.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = toolbar_webview.evaluate_script(&format!(
                            "if(window.applyTheme)applyTheme('{}')",
                            esc
                        ));
                    }
                    UserEvent::AddBookmark(title, url) => {
                        if !url.is_empty() && !url.starts_with("aurora://") {
                            crate::config::add_bookmark(title, url);
                            // Refresh toolbar bookmark bar with the new entry.
                            let bm_json = crate::config::load_bookmarks();
                            let _ = toolbar_webview.evaluate_script(&format!(
                                "if(window.refreshBookmarkBar){{window.AURORA_BOOKMARKS={};window.refreshBookmarkBar();}}",
                                bm_json
                            ));
                        }
                    }
                    UserEvent::ClearHistory => {
                        crate::history::clear_history();
                        // If history page is currently open, reload it to show empty state.
                        if current_url.starts_with("aurora://history") {
                            if let Some(ref cv) = content_webview_opt {
                                let _ = cv.load_html(&crate::history::get_history_html(&ipc_token));
                            }
                        }
                    }

                    UserEvent::UpdateUrl(url) => {
                        if !url.starts_with("aurora://") {
                            current_url = url.clone();
                        }
                        let escaped = url.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = toolbar_webview.evaluate_script(&format!("onUrlChanged('{}')", escaped));
                    }
                    UserEvent::UpdateTitle(title) => {
                        let escaped = title.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = toolbar_webview.evaluate_script(&format!("onTitleChanged('{}')", escaped));
                    }
                    UserEvent::LoadStart => {
                        let _ = toolbar_webview.evaluate_script("onLoadStart()");
                    }
                    UserEvent::LoadEnd => {
                        let _ = toolbar_webview.evaluate_script("onLoadEnd()");
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script(&format!(
                                "window.ipc.postMessage('{}:history:push:'+JSON.stringify({{title:document.title,url:location.href}}))",
                                ipc_token
                            ));
                        }
                    }

                    UserEvent::OpenDevTools => {
                        if let Some(ref cv) = content_webview_opt {
                            cv.open_devtools();
                        }
                    }
                    UserEvent::SetZoom(level) => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.zoom(*level);
                        }
                    }
                    UserEvent::FindText(text) => {
                        if let Some(ref cv) = content_webview_opt {
                            if text.is_empty() {
                                let _ = cv.evaluate_script("window.find && window.getSelection().removeAllRanges()");
                            } else {
                                let esc = text.replace('\\', "\\\\").replace('\'', "\\'");
                                let _ = cv.evaluate_script(&format!("window.find('{}', false, false, true)", esc));
                            }
                        }
                    }
                    UserEvent::FindPrev(text) => {
                        if let Some(ref cv) = content_webview_opt {
                            if !text.is_empty() {
                                let esc = text.replace('\\', "\\\\").replace('\'', "\\'");
                                let _ = cv.evaluate_script(&format!("window.find('{}', false, true, true)", esc));
                            }
                        }
                    }
                    UserEvent::Print => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script("window.print()");
                        }
                    }
                    UserEvent::ViewSource => {
                        if let Some(ref cv) = content_webview_opt {
                            let _ = cv.evaluate_script("location.href='view-source:'+location.href");
                        }
                    }

                    UserEvent::ToggleFullscreen => {
                        if window.fullscreen().is_some() {
                            window.set_fullscreen(None);
                        } else {
                            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                        }
                    }
                    UserEvent::MinimizeWindow     => { window.set_minimized(true); }
                    UserEvent::MaximizeWindow     => { window.set_maximized(!window.is_maximized()); }
                    UserEvent::CloseWindow        => {
                        // Servo's ServoInner::drop() spins the event loop synchronously
                        // which deadlocks when the embedder is also exiting. Hard-exit
                        // when Servo is loaded; the window content has nothing
                        // important to flush since we never made a session.
                        #[cfg(feature = "servo-engine")]
                        if servo_view.is_some() {
                            crate::servo_view::ServoView::force_exit();
                        }
                        *control_flow = ControlFlow::Exit;
                    }
                    UserEvent::DragWindow         => { let _ = window.drag_window(); }
                    UserEvent::SwitchEngine(engine) => {
                        // Save URL and engine to config BEFORE spawning new process
                        if !current_url.starts_with("aurora://") {
                            crate::config::set_last_url(&current_url);
                        }
                        crate::config::set_engine(engine);

                        if let Ok(exe) = std::env::current_exe() {
                            let mut cmd = Command::new(&exe);
                            cmd.arg(format!("--engine={}", engine));
                            if !current_url.starts_with("aurora://") {
                                cmd.arg(&current_url);
                            }
                            if let Ok(_child) = cmd.spawn() {
                                std::thread::sleep(std::time::Duration::from_millis(300));
                            }
                        }
                        #[cfg(feature = "servo-engine")]
                        if servo_view.is_some() {
                            crate::servo_view::ServoView::force_exit();
                        }
                        *control_flow = ControlFlow::Exit;
                    }

                    #[cfg(feature = "servo-engine")]
                    UserEvent::ServoWake => { /* handled above */ }
                    #[cfg(feature = "servo-engine")]
                    UserEvent::AuroraIpc(_) => { /* handled above */ }
                }
            }

            Event::WindowEvent { event: WindowEvent::Moved(_), .. } => {}
            _ => {}
        }
    });
}
