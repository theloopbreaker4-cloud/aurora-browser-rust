// app.rs — event loop, window creation, UserEvent dispatch
use crate::events::UserEvent;
use crate::ipc::generate_ipc_token;
use crate::settings;
use crate::webviews;
use std::process::Command;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Fullscreen, WindowBuilder};

// Total height of the toolbar area (tab bar + nav bar + bookmark bar)
const TOOLBAR_HEIGHT: u32 = 140;

pub fn run() {
    // Each session gets a unique token to prevent unauthorized IPC calls
    // from page scripts into Aurora's internal commands
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let ipc_token = generate_ipc_token();

    // Build the main window
    let mut wb = WindowBuilder::new()
        .with_title("Aurora Browser")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .with_decorations(true);
    if let Some(ico) = crate::icon::load_aurora_icon() {
        wb = wb.with_window_icon(Some(ico));
    }
    let window = wb.build(&event_loop).expect("Failed to create window");

    // Calculate initial bounds for both WebViews based on window size
    let size = window.inner_size().to_logical::<u32>(window.scale_factor());
    let toolbar_bounds = webviews::toolbar_bounds(size.width, TOOLBAR_HEIGHT);
    let content_bounds = webviews::content_bounds(size.width, size.height, TOOLBAR_HEIGHT);

    // Two child WebViews: toolbar (top) and content (rest of window)
    let toolbar_webview =
        webviews::build_toolbar_webview(&window, toolbar_bounds, proxy.clone());
    let portal_html = webviews::portal_html(&ipc_token);
    let content_webview = webviews::build_content_webview(
        &window,
        content_bounds,
        proxy.clone(),
        &ipc_token,
        &portal_html,
    );

    // Set initial address bar state to new tab
    let _ = proxy.send_event(UserEvent::UpdateUrl("aurora://newtab".to_string()));

    // Pre-generate portal HTML so it's available inside the move closure below
    let portal_html_for_loop = webviews::portal_html(&ipc_token);
    let proxy_for_events = proxy.clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }

            // Recalculate WebView bounds whenever the window is resized
            Event::WindowEvent {
                event: WindowEvent::Resized(new_size),
                ..
            } => {
                let s = new_size.to_logical::<u32>(window.scale_factor());
                let _ = toolbar_webview
                    .set_bounds(webviews::toolbar_bounds(s.width, TOOLBAR_HEIGHT));
                let _ = content_webview
                    .set_bounds(webviews::content_bounds(s.width, s.height, TOOLBAR_HEIGHT));
            }

            Event::UserEvent(ref user_event) => match user_event {
                // Route aurora:// URLs to internal pages, everything else to load_url
                UserEvent::Navigate(url) => {
                    if url == "aurora://newtab" || url == "aurora://portal" {
                        let _ = content_webview.load_html(&portal_html_for_loop);
                        let _ = proxy_for_events.send_event(UserEvent::UpdateUrl("aurora://newtab".to_string()));
                    } else if url == "aurora://settings" {
                        let html = settings::get_settings_html(&ipc_token);
                        let _ = content_webview.load_html(&html);
                        let _ = proxy_for_events.send_event(UserEvent::UpdateUrl(url.clone()));
                    } else {
                        let _ = content_webview.load_url(url);
                    }
                }

                UserEvent::GoBack => {
                    let _ = content_webview.evaluate_script("history.back()");
                }
                UserEvent::GoForward => {
                    let _ = content_webview.evaluate_script("history.forward()");
                }

                // If on the portal page reload navigates back to it instead of reloading data: URL
                UserEvent::Reload => {
                    let _ = content_webview.evaluate_script(&format!(
                        "if(document.title==='Aurora') window.ipc.postMessage('{}:navigate:aurora://newtab'); else location.reload();",
                        ipc_token
                    ));
                }

                UserEvent::Stop => {
                    let _ = content_webview.evaluate_script("window.stop()");
                }

                // Restart: spawn a new process then exit the current one
                UserEvent::Restart => {
                    if let Ok(exe) = std::env::current_exe() {
                        let mut cmd = Command::new(exe);
                        for arg in std::env::args().skip(1) {
                            cmd.arg(arg);
                        }
                        if cmd.spawn().is_err() {
                            eprintln!("Failed to restart Aurora");
                        }
                    } else {
                        eprintln!("Failed to resolve Aurora executable path");
                    }
                    *control_flow = ControlFlow::Exit;
                }

                // Escape special chars before injecting into JS string
                UserEvent::UpdateUrl(url) => {
                    let escaped = url.replace('\\', "\\\\").replace('\'', "\\'");
                    let _ = toolbar_webview
                        .evaluate_script(&format!("onUrlChanged('{}')", escaped));
                }
                UserEvent::UpdateTitle(title) => {
                    let escaped = title.replace('\\', "\\\\").replace('\'', "\\'");
                    let _ = toolbar_webview
                        .evaluate_script(&format!("onTitleChanged('{}')", escaped));
                }

                UserEvent::LoadStart => {
                    let _ = toolbar_webview.evaluate_script("onLoadStart()");
                }
                UserEvent::LoadEnd => {
                    let _ = toolbar_webview.evaluate_script("onLoadEnd()");
                }

                UserEvent::OpenDevTools => {
                    content_webview.open_devtools();
                }

                UserEvent::SetZoom(level) => {
                    let _ = content_webview.zoom(*level);
                }

                // window.find(text, caseSensitive, backwards, wrapAround)
                UserEvent::FindText(text) => {
                    if text.is_empty() {
                        let _ = content_webview.evaluate_script("window.find && window.getSelection().removeAllRanges()");
                    } else {
                        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
                        let _ = content_webview.evaluate_script(&format!("window.find('{}', false, false, true)", escaped));
                    }
                }

                UserEvent::Print => {
                    let _ = content_webview.evaluate_script("window.print()");
                }

                // Navigate to view-source: prefix to show raw HTML
                UserEvent::ViewSource => {
                    let _ = content_webview.evaluate_script(
                        "location.href = 'view-source:' + location.href"
                    );
                }

                // Toggle between borderless fullscreen and normal window
                UserEvent::ToggleFullscreen => {
                    if window.fullscreen().is_some() {
                        window.set_fullscreen(None);
                    } else {
                        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                    }
                }
            },
            _ => {}
        }
    });
}
