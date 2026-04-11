// portal.rs — builds new tab portal HTML with bookmarks and aurora icon
use crate::config;
use base64::Engine;

const ICON_PNG: &[u8] = include_bytes!("../icon.png");

pub fn get_portal_html(ipc_token: &str) -> String {
    let bookmarks_json = config::load_bookmarks();
    let icon_b64 = base64::engine::general_purpose::STANDARD.encode(ICON_PNG);
    let template = include_str!("portal.html");
    template
        .replace("__BOOKMARKS_JSON__", &bookmarks_json)
        .replace("__IPC_TOKEN__", ipc_token)
        .replace("__ICON_BASE64__", &icon_b64)
}
