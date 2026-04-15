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

    // Build the main window
    let mut wb = WindowBuilder::new()
        .with_title("Aurora Browser")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .with_decorations(false);
    if let Some(ico) = crate::icon::load_aurora_icon() {
        wb = wb.with_window_icon(Some(ico));
    }
    let window = wb.build(&event_loop).expect("Failed to create window");

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
        let url = if startup_url.starts_with("aurora://") {
            "https://www.google.com".to_string()
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
        ) {
            Ok(sv) => {
                let _ = proxy.send_event(UserEvent::UpdateUrl(url.clone()));
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

                // When Servo is active, forward navigation/input to it
                #[cfg(feature = "servo-engine")]
                if let Some(ref sv) = servo_view {
                    match user_event {
                        UserEvent::Navigate(url) if !url.starts_with("aurora://") => {
                            sv.navigate(url);
                            return;
                        }
                        UserEvent::GoBack    => { sv.go_back();    return; }
                        UserEvent::GoForward => { sv.go_forward(); return; }
                        UserEvent::Reload    => { sv.reload();     return; }
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
                    UserEvent::CloseWindow        => { *control_flow = ControlFlow::Exit; }
                    UserEvent::DragWindow         => { let _ = window.drag_window(); }
                    UserEvent::SwitchEngine(engine) => {
                        // Save URL and engine to config BEFORE spawning new process
                        if !current_url.starts_with("aurora://") {
                            crate::config::set_last_url(&current_url);
                        }
                        crate::config::set_engine(engine);

                        if let Ok(exe) = std::env::current_exe() {
                            let mut cmd = Command::new(&exe);
                            // Pass engine as --engine=servo so new process doesn't
                            // rely on reading config.json (avoids race condition on flush)
                            cmd.arg(format!("--engine={}", engine));
                            if !current_url.starts_with("aurora://") {
                                cmd.arg(&current_url);
                            }
                            let _ = cmd.spawn();
                        }
                        *control_flow = ControlFlow::Exit;
                    }

                    #[cfg(feature = "servo-engine")]
                    UserEvent::ServoWake => { /* handled above */ }
                }
            }

            Event::WindowEvent { event: WindowEvent::Moved(_), .. } => {}
            _ => {}
        }
    });
}
