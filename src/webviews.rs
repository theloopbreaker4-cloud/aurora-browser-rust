// webviews.rs — WebView construction and IPC message routing
use crate::config;
use crate::events::UserEvent;
use crate::history;
use crate::ipc::validate_ipc_message;
use crate::portal;
use crate::toolbar;
use std::fs;
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{PageLoadEvent, Rect, WebView, WebViewBuilder};

/// Builds the toolbar WebView (top strip: tabs, nav bar, bookmark bar).
/// Does NOT use an IPC token — toolbar is trusted internal UI.
pub fn build_toolbar_webview(
    window: &Window,
    bounds: Rect,
    proxy: EventLoopProxy<UserEvent>,
) -> WebView {
    let proxy_toolbar = proxy.clone();
    WebViewBuilder::new()
        .with_bounds(bounds)
        .with_html(&toolbar::get_toolbar_html())
        .with_transparent(true)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            // Toolbar sends raw messages without token prefix
            let msg = req.body().trim_matches('"').to_string();
            if let Some(url) = msg.strip_prefix("navigate:") {
                let _ = proxy_toolbar.send_event(UserEvent::Navigate(url.to_string()));
            } else if msg == "back" {
                let _ = proxy_toolbar.send_event(UserEvent::GoBack);
            } else if msg == "forward" {
                let _ = proxy_toolbar.send_event(UserEvent::GoForward);
            } else if msg == "reload" {
                let _ = proxy_toolbar.send_event(UserEvent::Reload);
            } else if msg == "stop" {
                let _ = proxy_toolbar.send_event(UserEvent::Stop);
            } else if let Some(action) = msg.strip_prefix("menu:") {
                // Menu items: map known actions to navigation or one-off events.
                // Unknown actions fall through to aurora://<action> for forward compatibility.
                let event = match action {
                    "bookmark_manager" | "import_export" | "bookmarks" => {
                        UserEvent::Navigate("aurora://bookmarks".to_string())
                    }
                    "recent_pages" | "all_history" | "history" => {
                        UserEvent::Navigate("aurora://history".to_string())
                    }
                    "all_downloads" | "downloads" => {
                        UserEvent::Navigate("aurora://downloads".to_string())
                    }
                    "general" | "privacy" | "settings" => {
                        UserEvent::Navigate("aurora://settings".to_string())
                    }
                    "extensions" | "extension_manager" | "extension_store" => {
                        UserEvent::Navigate("aurora://extensions".to_string())
                    }
                    "open_incognito" | "incognito" | "about_incognito" => {
                        UserEvent::Navigate("aurora://incognito".to_string())
                    }
                    "clear_history" => UserEvent::ClearHistory,
                    _ => UserEvent::Navigate(format!("aurora://{}", action)),
                };
                let _ = proxy_toolbar.send_event(event);
            } else if let Some(payload) = msg.strip_prefix("add_bookmark:") {
                // Format: "add_bookmark:<title>|<url>" — toolbar JS sends this with current tab.
                if let Some((t, u)) = payload.split_once('|') {
                    let _ = proxy_toolbar.send_event(UserEvent::AddBookmark(
                        t.to_string(),
                        u.to_string(),
                    ));
                }
            } else if msg == "devtools" {
                let _ = proxy_toolbar.send_event(UserEvent::OpenDevTools);
            } else if msg == "fullscreen" {
                let _ = proxy_toolbar.send_event(UserEvent::ToggleFullscreen);
            } else if msg == "minimize" {
                let _ = proxy_toolbar.send_event(UserEvent::MinimizeWindow);
            } else if msg == "maximize" {
                let _ = proxy_toolbar.send_event(UserEvent::MaximizeWindow);
            } else if msg == "close_window" {
                let _ = proxy_toolbar.send_event(UserEvent::CloseWindow);
            } else if msg == "drag_window" {
                let _ = proxy_toolbar.send_event(UserEvent::DragWindow);
            } else if let Some(engine) = msg.strip_prefix("switch_engine:") {
                let _ = proxy_toolbar.send_event(UserEvent::SwitchEngine(engine.to_string()));
            } else if msg == "print" {
                let _ = proxy_toolbar.send_event(UserEvent::Print);
            } else if let Some(z) = msg.strip_prefix("zoom:") {
                if let Ok(level) = z.parse::<f64>() {
                    let _ = proxy_toolbar.send_event(UserEvent::SetZoom(level));
                }
            } else if let Some(text) = msg.strip_prefix("find:") {
                let _ = proxy_toolbar.send_event(UserEvent::FindText(text.to_string()));
            } else if let Some(text) = msg.strip_prefix("findprev:") {
                let _ = proxy_toolbar.send_event(UserEvent::FindPrev(text.to_string()));
            }
        })
        .with_devtools(true)
        .build_as_child(window)
        .expect("Failed to create toolbar webview")
}

/// Builds the content WebView (main browsing area below the toolbar).
/// All IPC messages must be prefixed with the session token to prevent
/// web pages from calling Aurora's internal commands.
pub fn build_content_webview(
    window: &Window,
    bounds: Rect,
    proxy: EventLoopProxy<UserEvent>,
    ipc_token: &str,
    initial_html: &str,
) -> WebView {
    // Each proxy clone is moved into a different closure below
    let proxy_content = proxy.clone();
    let proxy_title = proxy.clone();
    let proxy_load = proxy.clone();
    let proxy_new_window = proxy;
    let token_prefix = format!("{}:", ipc_token);

    WebViewBuilder::new()
        .with_bounds(bounds)
        .with_html(initial_html)
        // Intercept target=_blank and other new-window requests — open in same view
        .with_new_window_req_handler(move |url| {
            let _ = proxy_new_window.send_event(UserEvent::Navigate(url));
            false // returning false prevents the new OS window from opening
        })
        // Push page title changes up to the toolbar tab label
        .with_document_title_changed_handler(move |title| {
            let _ = proxy_title.send_event(UserEvent::UpdateTitle(title));
        })
        // Update toolbar loading indicator and address bar on navigation
        .with_on_page_load_handler(move |event, url| {
            match event {
                PageLoadEvent::Started => {
                    let _ = proxy_load.send_event(UserEvent::LoadStart);
                }
                PageLoadEvent::Finished => {
                    let _ = proxy_load.send_event(UserEvent::LoadEnd);
                }
            }
            let _ = proxy_load.send_event(UserEvent::UpdateUrl(url));
        })
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let msg = req.body().trim_matches('"');
            // Reject messages that don't carry the session token
            let msg = match validate_ipc_message(msg, &token_prefix) {
                Some(msg) => msg,
                None => return,
            };
            if let Some(url) = msg.strip_prefix("navigate:") {
                let _ = proxy_content.send_event(UserEvent::Navigate(url.to_string()));
            } else if msg == "app:restart" {
                let _ = proxy_content.send_event(UserEvent::Restart);
            } else if msg == "devtools" {
                let _ = proxy_content.send_event(UserEvent::OpenDevTools);
            } else if msg == "print" {
                let _ = proxy_content.send_event(UserEvent::Print);
            } else if msg == "viewsource" {
                let _ = proxy_content.send_event(UserEvent::ViewSource);
            } else if msg == "fullscreen" {
                let _ = proxy_content.send_event(UserEvent::ToggleFullscreen);
            } else if let Some(z) = msg.strip_prefix("zoom:") {
                if let Ok(level) = z.parse::<f64>() {
                    let _ = proxy_content.send_event(UserEvent::SetZoom(level));
                }
            } else if let Some(text) = msg.strip_prefix("find:") {
                let _ = proxy_content.send_event(UserEvent::FindText(text.to_string()));
            } else if let Some(rest) = msg.strip_prefix("bookmark:add:") {
                // Format: "bookmark:add:<title>:<url>" (url itself may contain ':')
                if let Some((title, url)) = rest.split_once(':') {
                    config::add_bookmark(title, url);
                }
            } else if let Some(title) = msg.strip_prefix("bookmark:remove:") {
                config::remove_bookmark(title);
            } else if let Some(json) = msg.strip_prefix("bookmark:import:") {
                // Replace whole bookmark set from JSON payload (validated client-side).
                config::set_bookmarks_raw(json);
            } else if let Some(rest) = msg.strip_prefix("history:push:") {
                if let Ok(entry) = serde_json::from_str::<serde_json::Value>(rest) {
                    let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    history::push_history_entry(title, url);
                }
            } else if msg.starts_with("config:") {
                // Format: "config:key:value" — persisted to config.json next to exe
                let parts: Vec<&str> = msg.splitn(3, ':').collect();
                if parts.len() == 3 {
                    let dir = config::exe_dir();
                    let config_path = dir.join("config.json");
                    let config_str = fs::read_to_string(&config_path)
                        .or_else(|_| fs::read_to_string("config.json"))
                        .unwrap_or_else(|_| "{}".to_string());
                    let new_config = config::update_config_value(&config_str, parts[1], parts[2]);
                    // Write to both exe dir and cwd as fallback
                    let _ = fs::write(&config_path, &new_config);
                    let _ = fs::write("config.json", &new_config);
                    // Broadcast theme changes to the toolbar so the change is visible immediately.
                    if parts[1] == "theme" {
                        let _ = proxy_content
                            .send_event(UserEvent::ApplyTheme(parts[2].to_string()));
                    }
                }
            } else if let Some(engine) = msg.strip_prefix("switch_engine:") {
                let _ = proxy_content.send_event(UserEvent::SwitchEngine(engine.to_string()));
            }
        })
        .with_devtools(true)
        .build_as_child(window)
        .expect("Failed to create content webview")
}

/// Toolbar occupies the full width at the top of the window
pub fn toolbar_bounds(width: u32, height: u32) -> Rect {
    Rect {
        position: LogicalPosition::new(0, 0).into(),
        size: LogicalSize::new(width, height).into(),
    }
}

/// Content area starts directly below the toolbar
pub fn content_bounds(width: u32, height: u32, toolbar_height: u32) -> Rect {
    Rect {
        position: LogicalPosition::new(0, toolbar_height).into(),
        size: LogicalSize::new(width, height.saturating_sub(toolbar_height)).into(),
    }
}

pub fn portal_html(ipc_token: &str) -> String {
    portal::get_portal_html(ipc_token)
}
